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
pub struct AttemptId(pub(crate) u64);

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
    pub(crate) _private: (),
}

impl fmt::Debug for UnlockAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UnlockAuthorization(<redacted>)")
    }
}
