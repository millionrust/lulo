use rmac_updates::{
    Cancellation, Error, ErrorKind, InstallPlan, InstallProgress, InstallResult, Request, Snapshot,
};
#[cfg(target_os = "linux")]
use rmac_updates::{InstallCollector, PlanCollector};

#[cfg(target_os = "linux")]
use crate::api::{
    is_progress_change_member, CHECK_TIMEOUT, FILTER_NONE, FLAG_SIMULATE, INSTALL_STALL_TIMEOUT,
    INSTALL_TIMEOUT, OFFLINE_ACTION_REBOOT, OFFLINE_INTERFACE, PACKAGEKIT_DESTINATION,
    PACKAGEKIT_INTERFACE, PACKAGEKIT_PATH, POLL_INTERVAL, ROLE_UPDATE_PACKAGES, SIMULATION_TIMEOUT,
    TRANSACTION_INTERFACE,
};
use crate::api::{FLAG_ONLY_DOWNLOAD, FLAG_ONLY_TRUSTED};

/// `UpdatePackages` flags for an offline update's download: trusted packages
/// only, downloaded but not installed.
pub(crate) const fn offline_update_flags() -> u64 {
    FLAG_ONLY_TRUSTED | FLAG_ONLY_DOWNLOAD
}

/// A PackageKit `Details` size: the download size when it reports one,
/// otherwise the package size. Zero means unknown.
pub(crate) fn details_download_size(download: Option<u64>, size: Option<u64>) -> Option<u64> {
    download
        .filter(|value| *value > 0)
        .or(size.filter(|value| *value > 0))
}

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
    simulate_with(PlanCollector::new(snapshot)?, cancellation).await
}

#[cfg(target_os = "linux")]
async fn simulate_with(
    mut collector: PlanCollector,
    cancellation: &Cancellation,
) -> Result<InstallPlan, Error> {
    use futures_util::StreamExt as _;

    let ids = collector.requested_ids();
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
    run_update(plan, FLAG_ONLY_TRUSTED, true, cancellation, sender).await
}

/// Refresh, resolve the selection against the fresh update set (keeping
/// what is already prepared for restart), and simulate it.
#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_prepare_selection(
    selection: Vec<String>,
    cancellation: Cancellation,
) -> Result<(Snapshot, InstallPlan), Error> {
    let mut snapshot =
        packagekit_snapshot_with_cancellation(Request::refresh(), Some(&cancellation)).await?;
    if let Ok(connection) = packagekit_connection().await {
        snapshot.offline = offline_status(&connection).await.unwrap_or_default();
    }
    let requested = rmac_updates::resolve_selection(&snapshot, &selection)?;
    let plan = simulate_with(PlanCollector::for_updates(requested)?, &cancellation).await?;
    Ok((snapshot, plan))
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_prepare_selection(
    _selection: Vec<String>,
    _cancellation: Cancellation,
) -> Result<(Snapshot, InstallPlan), Error> {
    Err(unsupported())
}

/// Download the reviewed plan's updates for an offline update and trigger
/// it. Download-only transactions need no polkit authorization
/// (`pk_transaction_obtain_authorization`), and PackageKit's policy grants
/// the trigger to the active session's user, so no password prompt is
/// involved (docs/software-update.md "Authorization").
#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_prepare_offline(
    plan: InstallPlan,
    cancellation: Cancellation,
    sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("update download"));
    }
    let current_snapshot =
        packagekit_snapshot_with_cancellation(Request::refresh(), Some(&cancellation)).await?;
    let current_ids = current_snapshot.installable_ids();
    if plan
        .requested_ids()
        .iter()
        .any(|id| current_ids.binary_search(id).is_err())
    {
        return Err(Error::new(
            ErrorKind::Stale,
            "the available update set changed; review the new updates first",
        ));
    }
    let current_plan = simulate_with(
        PlanCollector::for_updates(plan.requested.clone())?,
        &cancellation,
    )
    .await?;
    if current_plan != plan {
        return Err(Error::new(
            ErrorKind::Stale,
            "the dependency plan changed; review the new updates first",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error("update download"));
    }
    let result = run_update(plan, offline_update_flags(), false, cancellation, sender).await?;
    let connection = packagekit_connection().await?;
    offline_proxy(&connection)
        .await?
        .call::<_, _, ()>("Trigger", &(OFFLINE_ACTION_REBOOT,))
        .await
        .map_err(|error| call_error("schedule the update for restart", &error))?;
    Ok(result)
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_prepare_offline(
    _plan: InstallPlan,
    _cancellation: Cancellation,
    _sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    Err(unsupported())
}

/// One `UpdatePackages` transaction for exactly the plan's requested IDs,
/// with progress, bounded time, and cancellation.
#[cfg(target_os = "linux")]
async fn run_update(
    plan: InstallPlan,
    flags: u64,
    interactive: bool,
    cancellation: Cancellation,
    sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    use futures_util::StreamExt as _;

    let ids = plan.requested_ids();
    let connection = packagekit_connection().await?;
    let (transaction, mut messages) = open_transaction(&connection, 4096).await?;
    set_hints(&transaction, 0, interactive).await?;
    let mut progress = InstallProgress::default();
    let _ = sender.try_send(progress.clone());
    transaction
        .call::<_, _, ()>("UpdatePackages", &(flags, ids))
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

/// The update set with download sizes, release notes, and the offline
/// update's state. Only the update set is required.
#[cfg(target_os = "linux")]
pub(crate) async fn packagekit_details_snapshot(request: Request) -> Result<Snapshot, Error> {
    let mut snapshot = packagekit_snapshot(request).await?;
    if let Ok(connection) = packagekit_connection().await {
        let ids = snapshot
            .updates
            .iter()
            .map(|update| update.package_id.clone())
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            if let Ok(sizes) = download_sizes(&connection, ids).await {
                snapshot.download_sizes = sizes;
            }
        }
        snapshot.offline = offline_status(&connection).await.unwrap_or_default();
    }
    snapshot.release_notes = snapshot
        .updates
        .iter()
        .find(|update| update.name == "rmac-session")
        .and_then(|update| {
            rmac_updates::lulo_release_notes(
                std::path::Path::new(rmac_updates::APT_LISTS_DIR),
                update,
            )
        });
    Ok(snapshot)
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn packagekit_details_snapshot(request: Request) -> Result<Snapshot, Error> {
    packagekit_snapshot(request).await
}

/// `GetDetails` for `ids`: each package's download size where known.
#[cfg(target_os = "linux")]
async fn download_sizes(
    connection: &zbus::Connection,
    ids: Vec<String>,
) -> Result<std::collections::BTreeMap<String, u64>, Error> {
    use futures_util::StreamExt as _;
    use zbus::zvariant::OwnedValue;

    let wanted = ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let (transaction, mut messages) = open_transaction(connection, 2048).await?;
    set_hints(&transaction, 3600, false).await?;
    transaction
        .call::<_, _, ()>("GetDetails", &(ids,))
        .await
        .map_err(|error| call_error("read update sizes", &error))?;
    let timeout = futures_util::FutureExt::fuse(async_io::Timer::after(CHECK_TIMEOUT));
    futures_util::pin_mut!(timeout);
    let mut sizes = std::collections::BTreeMap::new();
    loop {
        futures_util::select! {
            message = messages.next() => {
                let message = message
                    .ok_or_else(|| protocol_error("the details transaction ended unexpectedly"))?
                    .map_err(|_| protocol_error("could not read update details"))?;
                let header = message.header();
                match header.member().map(|member| member.as_str().to_owned()).as_deref() {
                    Some("Details") => {
                        let (data,): (std::collections::HashMap<String, OwnedValue>,) = message
                            .body()
                            .deserialize()
                            .map_err(|_| protocol_error("invalid update details"))?;
                        let text = |key: &str| {
                            data.get(key)
                                .and_then(|value| value.try_clone().ok())
                                .and_then(|value| String::try_from(value).ok())
                        };
                        let number = |key: &str| {
                            data.get(key)
                                .and_then(|value| value.try_clone().ok())
                                .and_then(|value| u64::try_from(value).ok())
                        };
                        if let Some(id) = text("package-id").filter(|id| wanted.contains(id)) {
                            if let Some(size) =
                                details_download_size(number("download-size"), number("size"))
                            {
                                sizes.insert(id, size);
                            }
                        }
                    }
                    Some("Finished") => return Ok(sizes),
                    _ => {}
                }
            }
            _ = timeout => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(Error::new(ErrorKind::Timeout, "reading update sizes timed out"));
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn offline_proxy(connection: &zbus::Connection) -> Result<zbus::Proxy<'_>, Error> {
    zbus::Proxy::new(
        connection,
        PACKAGEKIT_DESTINATION,
        PACKAGEKIT_PATH,
        OFFLINE_INTERFACE,
    )
    .await
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system update service is unavailable",
        )
    })
}

/// The prepared offline update. `GetPrepared` fails when nothing is
/// prepared, which is an empty set, not an error.
#[cfg(target_os = "linux")]
async fn offline_status(
    connection: &zbus::Connection,
) -> Result<rmac_updates::OfflineStatus, Error> {
    let proxy = offline_proxy(connection).await?;
    let prepared_flag = proxy
        .get_property::<bool>("UpdatePrepared")
        .await
        .map_err(|_| protocol_error("could not read the prepared update"))?;
    let triggered = proxy
        .get_property::<bool>("UpdateTriggered")
        .await
        .map_err(|_| protocol_error("could not read the prepared update"))?;
    let mut prepared = if prepared_flag {
        proxy
            .call::<_, _, Vec<String>>("GetPrepared", &())
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    prepared.retain(|id| rmac_updates::Update::from_packagekit(0, id, "").is_some());
    prepared.sort();
    prepared.dedup();
    prepared.truncate(rmac_updates::MAX_PLAN_CHANGES);
    Ok(rmac_updates::OfflineStatus {
        triggered: triggered && !prepared.is_empty(),
        prepared,
    })
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "software updates are available in the supported Linux session",
    )
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
