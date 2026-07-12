//! Fail-closed process lifecycle around the Wayland/PAM runtime.
//!
//! This module is deliberately free of systemd, logind, Wayland, and PAM I/O
//! so readiness and advisory-hint ordering can be tested on every host.

use std::fmt;

use crate::runtime::ProviderExit;

#[derive(Default)]
pub(crate) struct ProcessLifecycle {
    ready: bool,
    terminal: bool,
}

impl ProcessLifecycle {
    pub(crate) fn apply(
        &mut self,
        notify_ready: bool,
        exit: Option<ProviderExit>,
    ) -> Result<ProcessActions, Error> {
        if self.terminal {
            return Err(Error::StatusAfterExit);
        }
        if notify_ready && self.ready {
            return Err(Error::DuplicateReady);
        }
        if notify_ready && matches!(exit, Some(ProviderExit::AuthenticatedUnlock)) {
            return Err(Error::InvalidCombinedStatus);
        }

        let was_ready = self.ready;
        self.ready |= notify_ready;
        let mut actions = ProcessActions {
            locked_hint: notify_ready.then_some(true),
            notify_systemd_ready: notify_ready && exit.is_none(),
            termination: ProcessTermination::Continue,
        };

        match exit {
            None => {}
            Some(ProviderExit::AuthenticatedUnlock) if self.ready => {
                actions.locked_hint = Some(false);
                actions.notify_systemd_ready = false;
                actions.termination = ProcessTermination::AuthenticatedUnlock;
                self.terminal = true;
            }
            Some(ProviderExit::AuthenticatedUnlock) => return Err(Error::ExitBeforeReady),
            Some(ProviderExit::Denied) if !was_ready && !notify_ready => {
                actions.termination = ProcessTermination::RestartRequired;
                self.terminal = true;
            }
            Some(ProviderExit::Denied) => return Err(Error::DeniedAfterReady),
            Some(ProviderExit::FailedLocked) if self.ready => {
                actions.notify_systemd_ready = false;
                actions.termination = ProcessTermination::RestartRequired;
                self.terminal = true;
            }
            Some(ProviderExit::FailedLocked) => return Err(Error::FailureBeforeReady),
        }
        Ok(actions)
    }
}

impl fmt::Debug for ProcessLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessLifecycle")
            .field("ready", &self.ready)
            .field("terminal", &self.terminal)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessActions {
    pub(crate) locked_hint: Option<bool>,
    pub(crate) notify_systemd_ready: bool,
    pub(crate) termination: ProcessTermination,
}

pub(crate) trait ProcessEffects {
    fn set_locked_hint(&mut self, locked: bool) -> Result<(), ()>;
    fn notify_ready(&mut self) -> Result<(), ()>;
}

pub(crate) fn execute_actions(
    actions: ProcessActions,
    effects: &mut impl ProcessEffects,
) -> Result<ExecutedActions, Error> {
    let locked_hint_failed = actions
        .locked_hint
        .is_some_and(|locked| effects.set_locked_hint(locked).is_err());
    if actions.notify_systemd_ready {
        effects.notify_ready().map_err(|()| Error::NotifyReady)?;
    }
    Ok(ExecutedActions {
        locked_hint_failed,
        termination: actions.termination,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExecutedActions {
    pub(crate) locked_hint_failed: bool,
    pub(crate) termination: ProcessTermination,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessTermination {
    Continue,
    AuthenticatedUnlock,
    RestartRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    DuplicateReady,
    StatusAfterExit,
    InvalidCombinedStatus,
    ExitBeforeReady,
    DeniedAfterReady,
    FailureBeforeReady,
    NotifyReady,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("secure lock process lifecycle failed")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingEffects {
        events: Vec<&'static str>,
        fail_hint: bool,
        fail_notify: bool,
    }

    impl ProcessEffects for RecordingEffects {
        fn set_locked_hint(&mut self, locked: bool) -> Result<(), ()> {
            self.events
                .push(if locked { "hint:true" } else { "hint:false" });
            if self.fail_hint {
                Err(())
            } else {
                Ok(())
            }
        }

        fn notify_ready(&mut self) -> Result<(), ()> {
            self.events.push("ready");
            if self.fail_notify {
                Err(())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn readiness_sets_hint_before_notifying_and_authenticated_exit_clears_it() {
        let mut lifecycle = ProcessLifecycle::default();
        let ready = lifecycle.apply(true, None).unwrap();
        assert_eq!(ready.locked_hint, Some(true));
        assert!(ready.notify_systemd_ready);
        assert_eq!(ready.termination, ProcessTermination::Continue);

        let unlocked = lifecycle
            .apply(false, Some(ProviderExit::AuthenticatedUnlock))
            .unwrap();
        assert_eq!(unlocked.locked_hint, Some(false));
        assert!(!unlocked.notify_systemd_ready);
        assert_eq!(
            unlocked.termination,
            ProcessTermination::AuthenticatedUnlock
        );
        assert_eq!(lifecycle.apply(false, None), Err(Error::StatusAfterExit));
    }

    #[test]
    fn denial_never_claims_readiness_or_changes_the_hint() {
        let mut lifecycle = ProcessLifecycle::default();
        let denied = lifecycle.apply(false, Some(ProviderExit::Denied)).unwrap();
        assert_eq!(denied.locked_hint, None);
        assert!(!denied.notify_systemd_ready);
        assert_eq!(denied.termination, ProcessTermination::RestartRequired);
    }

    #[test]
    fn failure_after_lock_never_clears_the_hint_or_notifies_ready() {
        let mut lifecycle = ProcessLifecycle::default();
        lifecycle.apply(true, None).unwrap();
        let failed = lifecycle
            .apply(false, Some(ProviderExit::FailedLocked))
            .unwrap();
        assert_eq!(failed.locked_hint, None);
        assert!(!failed.notify_systemd_ready);
        assert_eq!(failed.termination, ProcessTermination::RestartRequired);
    }

    #[test]
    fn impossible_exit_orderings_fail_closed() {
        let mut before_ready = ProcessLifecycle::default();
        assert_eq!(
            before_ready.apply(false, Some(ProviderExit::AuthenticatedUnlock)),
            Err(Error::ExitBeforeReady)
        );
        assert_eq!(
            before_ready.apply(false, Some(ProviderExit::FailedLocked)),
            Err(Error::FailureBeforeReady)
        );

        let mut ready = ProcessLifecycle::default();
        ready.apply(true, None).unwrap();
        assert_eq!(ready.apply(true, None), Err(Error::DuplicateReady));
        assert_eq!(
            ready.apply(false, Some(ProviderExit::Denied)),
            Err(Error::DeniedAfterReady)
        );
    }

    #[test]
    fn effects_set_the_hint_before_readiness_and_notification_failure_is_fatal() {
        let mut lifecycle = ProcessLifecycle::default();
        let ready = lifecycle.apply(true, None).unwrap();
        let mut effects = RecordingEffects::default();
        let executed = execute_actions(ready, &mut effects).unwrap();
        assert_eq!(effects.events, ["hint:true", "ready"]);
        assert!(!executed.locked_hint_failed);

        let mut failing = RecordingEffects {
            fail_hint: true,
            fail_notify: true,
            ..RecordingEffects::default()
        };
        assert_eq!(
            execute_actions(ready, &mut failing),
            Err(Error::NotifyReady)
        );
        assert_eq!(failing.events, ["hint:true", "ready"]);
    }
}
