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
