//! Bounded PackageKit update-status adapter for the supported Linux session.

use rmac_updates::{Error, ErrorKind, Request, Snapshot, SnapshotFuture, Source};

#[cfg(target_os = "linux")]
const PACKAGEKIT_DESTINATION: &str = "org.freedesktop.PackageKit";
#[cfg(target_os = "linux")]
const PACKAGEKIT_PATH: &str = "/org/freedesktop/PackageKit";
#[cfg(target_os = "linux")]
const PACKAGEKIT_INTERFACE: &str = "org.freedesktop.PackageKit";
#[cfg(target_os = "linux")]
const TRANSACTION_INTERFACE: &str = "org.freedesktop.PackageKit.Transaction";
#[cfg(target_os = "linux")]
const TRANSACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSource;

impl Source for SystemSource {
    fn snapshot(&self, request: Request) -> SnapshotFuture<'_> {
        Box::pin(packagekit_snapshot(request))
    }
}

pub async fn snapshot(request: Request) -> Result<Snapshot, Error> {
    SystemSource.snapshot(request).await
}

#[cfg(target_os = "linux")]
async fn packagekit_snapshot(request: Request) -> Result<Snapshot, Error> {
    use futures_util::StreamExt as _;
    use zbus::{message::Type, MatchRule, MessageStream, Proxy};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system update service is unavailable",
        )
    })?;
    let root = Proxy::new(
        &connection,
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
    })?;
    let transaction_path: zbus::zvariant::OwnedObjectPath = root
        .call("GetTid", &())
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not start an update check"))?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(transaction_path.as_str())
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid update transaction path"))?
        .interface(TRANSACTION_INTERFACE)
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid update transaction interface"))?
        .build();
    let mut messages = MessageStream::for_match_rule(rule, &connection, Some(1024))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch the update check"))?
        .fuse();
    let transaction = Proxy::new(
        &connection,
        PACKAGEKIT_DESTINATION,
        transaction_path.as_str(),
        TRANSACTION_INTERFACE,
    )
    .await
    .map_err(|_| Error::new(ErrorKind::Protocol, "could not open the update transaction"))?;
    let hints = vec![
        "locale=C".to_string(),
        "background=false".to_string(),
        "interactive=false".to_string(),
        format!("cache-age={}", request.cache_age_seconds),
    ];
    transaction
        .call::<_, _, ()>("SetHints", &(hints,))
        .await
        .map_err(|_| Error::new(ErrorKind::Protocol, "could not configure the update check"))?;
    transaction
        .call::<_, _, ()>("GetUpdates", &("none",))
        .await
        .map_err(|_| Error::new(ErrorKind::Backend, "could not query available updates"))?;

    let timeout = futures_util::FutureExt::fuse(async_io::Timer::after(TRANSACTION_TIMEOUT));
    futures_util::pin_mut!(timeout);
    let mut collector = rmac_updates::Collector::default();
    loop {
        futures_util::select! {
            message = messages.next() => {
                let message = message
                    .ok_or_else(|| Error::new(ErrorKind::Protocol, "the update transaction ended unexpectedly"))?
                    .map_err(|_| Error::new(ErrorKind::Protocol, "could not read the update transaction"))?;
                if let Some(event) = event_from_message(&message)? {
                    let finished = matches!(event, rmac_updates::Event::Finished { .. });
                    collector.apply(event)?;
                    if finished {
                        return collector.finish();
                    }
                }
            }
            _ = timeout => {
                let _ = transaction.call::<_, _, ()>("Cancel", &()).await;
                return Err(Error::new(ErrorKind::Timeout, "the update check timed out"));
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn packagekit_snapshot(_request: Request) -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "software update checks are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn event_from_message(message: &zbus::Message) -> Result<Option<rmac_updates::Event>, Error> {
    let header = message.header();
    let Some(member) = header.member() else {
        return Ok(None);
    };
    match member.as_str() {
        "Package" => {
            let (info, package_id, summary): (String, String, String) = message
                .body()
                .deserialize()
                .map_err(|_| Error::new(ErrorKind::Protocol, "invalid update package data"))?;
            Ok(Some(rmac_updates::Event::Package {
                info,
                package_id,
                summary,
            }))
        }
        "ErrorCode" => {
            let (code, detail): (String, String) = message
                .body()
                .deserialize()
                .map_err(|_| Error::new(ErrorKind::Protocol, "invalid update error data"))?;
            Ok(Some(rmac_updates::Event::BackendError { code, detail }))
        }
        "Finished" => {
            let (exit, _runtime): (String, u32) = message
                .body()
                .deserialize()
                .map_err(|_| Error::new(ErrorKind::Protocol, "invalid update completion data"))?;
            Ok(Some(rmac_updates::Event::Finished { exit }))
        }
        _ => Ok(None),
    }
}
