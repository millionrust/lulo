use rmac_updates::{
    Cancellation, Error, ErrorKind, InstallPlan, InstallProgress, InstallResult, Request, Snapshot,
};
#[cfg(target_os = "linux")]
use rmac_updates::{InstallCollector, PlanCollector};

#[cfg(target_os = "linux")]
use crate::api::{
    is_progress_change_member, CHECK_TIMEOUT, FILTER_NONE, FLAG_ONLY_TRUSTED, FLAG_SIMULATE,
    INSTALL_STALL_TIMEOUT, INSTALL_TIMEOUT, PACKAGEKIT_DESTINATION, PACKAGEKIT_INTERFACE,
    PACKAGEKIT_PATH, POLL_INTERVAL, ROLE_UPDATE_PACKAGES, SIMULATION_TIMEOUT,
    TRANSACTION_INTERFACE,
};

#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_snapshot(request: Request) -> Result<Snapshot, Error> {
    packagekit_snapshot_with_cancellation(request, None).await
}

#[cfg(target_os = "linux")]
async fn packagekit_snapshot_with_cancellation(
    request: Request,
    cancellation: Option<&Cancellation>,
) -> Result<Snapshot, Error> {
    use futures_util::StreamExt as _;

    if cancellation.is_some_and(Cancellation::is_cancelled) {
        return Err(cancelled_error("update check"));
    }
    let connection = packagekit_connection().await?;
    let root = packagekit_root(&connection).await?;
    let roles = root.get_property::<u64>("Roles").await.ok();
    let (transaction, mut messages) = open_transaction(&connection, 2048).await?;
    set_hints(&transaction, request.cache_age_seconds, false).await?;
    transaction
        .call::<_, _, ()>("GetUpdates", &(FILTER_NONE,))
        .await
        .map_err(|error| call_error("query available updates", &error))?;

    let timeout = futures_util::FutureExt::fuse(async_io::Timer::after(CHECK_TIMEOUT));
    futures_util::pin_mut!(timeout);
    let cancelled = futures_util::FutureExt::fuse(async {
        if let Some(cancellation) = cancellation {
            loop {
                if cancellation.is_cancelled() {
                    break;
                }
                async_io::Timer::after(POLL_INTERVAL).await;
            }
        } else {
            futures_util::future::pending::<()>().await;
        }
    });
    futures_util::pin_mut!(cancelled);
    let mut collector = rmac_updates::Collector::default();
    loop {
        futures_util::select! {
            message = messages.next() => {
                let message = message
                    .ok_or_else(|| protocol_error("the update transaction ended unexpectedly"))?
                    .map_err(|_| protocol_error("could not read the update transaction"))?;
                if let Some(event) = event_from_message(&message)? {
                    let finished = matches!(event, rmac_updates::Event::Finished { .. });
                    collector.apply(event)?;
                    if finished {
                        let mut snapshot = collector.finish()?;
                        snapshot.install_supported = roles
                            .is_some_and(|roles| roles & ROLE_UPDATE_PACKAGES != 0);
                        if !snapshot.install_supported {
                            snapshot.install_unavailable_reason = Some(
                                if roles.is_some() {
                                    "The PackageKit backend does not advertise package updates."
                                } else {
                                    "PackageKit did not expose its update capabilities."
                                }
                                .into(),
                            );
                        }
                        return Ok(snapshot);
                    }
                }
            }
            _ = timeout => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(Error::new(ErrorKind::Timeout, "the update check timed out"));
            }
            _ = cancelled => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(cancelled_error("update check"));
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_snapshot(_request: Request) -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "software updates are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_prepare(
    cancellation: Cancellation,
) -> Result<(Snapshot, InstallPlan), Error> {
    let snapshot =
        packagekit_snapshot_with_cancellation(Request::refresh(), Some(&cancellation)).await?;
    let plan = simulate(&snapshot, &cancellation).await?;
    Ok((snapshot, plan))
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_prepare(
    _cancellation: Cancellation,
) -> Result<(Snapshot, InstallPlan), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "software updates are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
async fn simulate(snapshot: &Snapshot, cancellation: &Cancellation) -> Result<InstallPlan, Error> {
    use futures_util::StreamExt as _;

    let mut collector = PlanCollector::new(snapshot)?;
    let ids = snapshot.installable_ids();
    let connection = packagekit_connection().await?;
    let root = packagekit_root(&connection).await?;
    let roles = root
        .get_property::<u64>("Roles")
        .await
        .map_err(|_| protocol_error("could not revalidate PackageKit capabilities"))?;
    if roles & ROLE_UPDATE_PACKAGES == 0 {
        return Err(Error::new(
            ErrorKind::Unavailable,
            "the PackageKit backend no longer advertises package updates",
        ));
    }
    let (transaction, mut messages) = open_transaction(&connection, 4096).await?;
    set_hints(&transaction, 0, false).await?;
    transaction
        .call::<_, _, ()>("UpdatePackages", &(FLAG_ONLY_TRUSTED | FLAG_SIMULATE, ids))
        .await
        .map_err(|error| call_error("simulate trusted updates", &error))?;

    let timeout = futures_util::FutureExt::fuse(async_io::Timer::after(SIMULATION_TIMEOUT));
    futures_util::pin_mut!(timeout);
    let mut cancel_sent = false;
    loop {
        let tick = futures_util::FutureExt::fuse(async_io::Timer::after(POLL_INTERVAL));
        futures_util::pin_mut!(tick);
        futures_util::select! {
            message = messages.next() => {
                let message = message
                    .ok_or_else(|| protocol_error("the update simulation ended unexpectedly"))?
                    .map_err(|_| protocol_error("could not read the update simulation"))?;
                if let Some(event) = event_from_message(&message)? {
                    let finished = matches!(event, rmac_updates::Event::Finished { .. });
                    collector.apply(event)?;
                    if finished {
                        return collector.finish();
                    }
                }
            }
            _ = tick => {
                if cancellation.is_cancelled() && !cancel_sent {
                    transaction
                        .call::<_, _, ()>("Cancel", &())
                        .await
                        .map_err(|error| call_error("cancel update preparation", &error))?;
                    cancel_sent = true;
                }
            }
            _ = timeout => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(Error::new(ErrorKind::Timeout, "the update simulation timed out"));
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_install(
    plan: InstallPlan,
    cancellation: Cancellation,
    sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    use futures_util::StreamExt as _;

    if cancellation.is_cancelled() {
        return Err(cancelled_error("update installation"));
    }
    let current_snapshot =
        packagekit_snapshot_with_cancellation(Request::refresh(), Some(&cancellation)).await?;
    if current_snapshot.installable_ids() != plan.requested_ids() {
        return Err(Error::new(
            ErrorKind::Stale,
            "the available update set changed; review the new plan before installing",
        ));
    }
    let current_plan = simulate(&current_snapshot, &cancellation).await?;
    if current_plan != plan {
        return Err(Error::new(
            ErrorKind::Stale,
            "the dependency plan changed; review the new plan before installing",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error("update installation"));
    }

    let ids = plan.requested_ids();
    let connection = packagekit_connection().await?;
    let (transaction, mut messages) = open_transaction(&connection, 4096).await?;
    set_hints(&transaction, 0, true).await?;
    let mut progress = InstallProgress::default();
    let _ = sender.try_send(progress.clone());
    transaction
        .call::<_, _, ()>("UpdatePackages", &(FLAG_ONLY_TRUSTED, ids))
        .await
        .map_err(|error| call_error("install trusted updates", &error))?;

    let timeout = futures_util::FutureExt::fuse(async_io::Timer::after(INSTALL_TIMEOUT));
    futures_util::pin_mut!(timeout);
    let mut last_activity = std::time::Instant::now();
    let mut cancel_sent = false;
    let mut user_cancel_attempted = false;
    let mut stall_cancel_attempted = false;
    let mut collector = InstallCollector::default();
    loop {
        let tick = futures_util::FutureExt::fuse(async_io::Timer::after(POLL_INTERVAL));
        futures_util::pin_mut!(tick);
        futures_util::select! {
            message = messages.next() => {
                let message = message
                    .ok_or_else(|| protocol_error("the update installation ended unexpectedly"))?
                    .map_err(|_| protocol_error("could not read the update installation"))?;
                last_activity = std::time::Instant::now();
                if message
                    .header()
                    .member()
                    .is_some_and(|member| is_progress_change_member(member.as_str()))
                {
                    refresh_progress(&transaction, &mut progress).await?;
                    let _ = sender.try_send(progress.clone());
                }
                if let Some(event) = event_from_message(&message)? {
                    let finished = matches!(event, rmac_updates::Event::Finished { .. });
                    match &event {
                        rmac_updates::Event::Package { package_id, .. } => {
                            progress.set_current_package(package_id);
                        }
                        rmac_updates::Event::RestartRequired { kind, .. } => {
                            progress.restart = progress
                                .restart
                                .max(rmac_updates::RestartRequirement::from_packagekit(*kind));
                        }
                        _ => {}
                    }
                    collector.apply(event)?;
                    if finished {
                        let result = collector.finish()?;
                        progress.phase = rmac_updates::InstallPhase::Complete;
                        progress.percentage = Some(100);
                        progress.allow_cancel = false;
                        progress.restart = result.restart;
                        let _ = sender.try_send(progress);
                        return Ok(result);
                    }
                    let _ = sender.try_send(progress.clone());
                }
            }
            _ = tick => {
                let cancellation_requested = cancellation.is_cancelled();
                let stalled = last_activity.elapsed() >= INSTALL_STALL_TIMEOUT;
                let should_attempt_cancel = !cancel_sent
                    && progress.allow_cancel
                    && ((cancellation_requested && !user_cancel_attempted)
                        || (stalled && !stall_cancel_attempted));
                if should_attempt_cancel {
                    user_cancel_attempted |= cancellation_requested;
                    stall_cancel_attempted |= stalled;
                    match transaction.call::<_, _, ()>("Cancel", &()).await {
                        Ok(()) => {
                            cancel_sent = true;
                            progress.phase = rmac_updates::InstallPhase::Cancelling;
                            progress.allow_cancel = false;
                            let _ = sender.try_send(progress.clone());
                        }
                        Err(_) => {
                            // AllowCancel can change between the property signal and
                            // this call. Re-read authority and keep owning the live
                            // transaction rather than treating that race as completion.
                            refresh_progress(&transaction, &mut progress).await?;
                            last_activity = std::time::Instant::now();
                            let _ = sender.try_send(progress.clone());
                        }
                    }
                }
            }
            _ = timeout => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(Error::new(
                    ErrorKind::Timeout,
                    "the update installation exceeded four hours",
                ));
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_install(
    _plan: InstallPlan,
    _cancellation: Cancellation,
    _sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "software updates are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
async fn packagekit_connection() -> Result<zbus::Connection, Error> {
    zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system update service is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
async fn packagekit_root(connection: &zbus::Connection) -> Result<zbus::Proxy<'_>, Error> {
    zbus::Proxy::new(
        connection,
        PACKAGEKIT_DESTINATION,
        PACKAGEKIT_PATH,
        PACKAGEKIT_INTERFACE,
    )
    .await
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system update service is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
async fn open_transaction<'a>(
    connection: &'a zbus::Connection,
    capacity: usize,
) -> Result<(zbus::Proxy<'a>, zbus::MessageStream), Error> {
    use zbus::{message::Type, MatchRule, MessageStream};

    let root = packagekit_root(connection).await?;
    let transaction_path: zbus::zvariant::OwnedObjectPath = root
        .call("CreateTransaction", &())
        .await
        .map_err(|error| call_error("create an update transaction", &error))?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(PACKAGEKIT_DESTINATION)
        .map_err(|_| protocol_error("invalid PackageKit signal sender"))?
        .path(transaction_path.as_str())
        .map_err(|_| protocol_error("invalid update transaction path"))?
        .build();
    let messages = MessageStream::for_match_rule(rule, connection, Some(capacity))
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not watch the update transaction",
            )
        })?;
    let transaction = zbus::Proxy::new(
        connection,
        PACKAGEKIT_DESTINATION,
        transaction_path,
        TRANSACTION_INTERFACE,
    )
    .await
    .map_err(|_| protocol_error("could not open the update transaction"))?;
    Ok((transaction, messages))
}

#[cfg(target_os = "linux")]
async fn set_hints(
    transaction: &zbus::Proxy<'_>,
    cache_age_seconds: u32,
    interactive: bool,
) -> Result<(), Error> {
    let hints = vec![
        "locale=C".to_string(),
        "background=false".to_string(),
        format!("interactive={interactive}"),
        format!("cache-age={cache_age_seconds}"),
    ];
    transaction
        .call::<_, _, ()>("SetHints", &(hints,))
        .await
        .map_err(|_| protocol_error("could not configure the update transaction"))
}

#[cfg(target_os = "linux")]
async fn refresh_progress(
    transaction: &zbus::Proxy<'_>,
    progress: &mut InstallProgress,
) -> Result<(), Error> {
    let status = transaction
        .get_property::<u32>("Status")
        .await
        .map_err(|_| protocol_error("could not read update status"))?;
    let percentage = transaction
        .get_property::<u32>("Percentage")
        .await
        .map_err(|_| protocol_error("could not read update progress"))?;
    let allow_cancel = transaction
        .get_property::<bool>("AllowCancel")
        .await
        .map_err(|_| protocol_error("could not read update cancellation state"))?;
    let remaining = transaction
        .get_property::<u32>("RemainingTime")
        .await
        .map_err(|_| protocol_error("could not read update remaining time"))?;
    progress.phase = rmac_updates::InstallPhase::from_packagekit(status);
    progress.set_percentage(percentage);
    progress.allow_cancel = allow_cancel;
    progress.remaining_seconds = (remaining > 0).then_some(remaining);
    Ok(())
}

#[cfg(target_os = "linux")]
fn event_from_message(message: &zbus::Message) -> Result<Option<rmac_updates::Event>, Error> {
    let header = message.header();
    let Some(member) = header.member() else {
        return Ok(None);
    };
    match member.as_str() {
        "Package" => {
            let (info, package_id, summary): (u32, String, String) =
                message
                    .body()
                    .deserialize()
                    .map_err(|_| protocol_error("invalid update package data"))?;
            Ok(Some(rmac_updates::Event::Package {
                info,
                package_id,
                summary,
            }))
        }
        "ErrorCode" => {
            let (code, detail): (u32, String) = message
                .body()
                .deserialize()
                .map_err(|_| protocol_error("invalid update error data"))?;
            Ok(Some(rmac_updates::Event::BackendError { code, detail }))
        }
        "Finished" => {
            let (exit, _runtime): (u32, u32) = message
                .body()
                .deserialize()
                .map_err(|_| protocol_error("invalid update completion data"))?;
            Ok(Some(rmac_updates::Event::Finished { exit }))
        }
        "RequireRestart" => {
            let (kind, package_id): (u32, String) = message
                .body()
                .deserialize()
                .map_err(|_| protocol_error("invalid update restart data"))?;
            Ok(Some(rmac_updates::Event::RestartRequired {
                kind,
                package_id,
            }))
        }
        "RepoSignatureRequired" => Ok(Some(rmac_updates::Event::BackendError {
            code: 31,
            detail: String::new(),
        })),
        "EulaRequired" => Ok(Some(rmac_updates::Event::BackendError {
            code: 34,
            detail: String::new(),
        })),
        "MediaChangeRequired" => Ok(Some(rmac_updates::Event::BackendError {
            code: 47,
            detail: String::new(),
        })),
        _ => Ok(None),
    }
}

#[cfg(target_os = "linux")]
fn call_error(operation: &'static str, error: &zbus::Error) -> Error {
    let detail = error.to_string().to_ascii_lowercase();
    if detail.contains("denied")
        || detail.contains("notauthorized")
        || detail.contains("not authorized")
        || detail.contains("polkit")
    {
        Error::new(
            ErrorKind::Authorization,
            format!("could not {operation}: authorization was denied or cancelled"),
        )
    } else if detail.contains("serviceunknown") || detail.contains("namehasnoowner") {
        Error::new(
            ErrorKind::Unavailable,
            format!("could not {operation}: PackageKit is unavailable"),
        )
    } else {
        Error::new(
            ErrorKind::Backend,
            format!("could not {operation}: PackageKit rejected the request"),
        )
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn protocol_error(detail: &'static str) -> Error {
    Error::new(ErrorKind::Protocol, detail)
}

#[cfg(target_os = "linux")]
fn cancelled_error(operation: &'static str) -> Error {
    Error::new(
        ErrorKind::Cancelled,
        format!("the {operation} was cancelled"),
    )
}
