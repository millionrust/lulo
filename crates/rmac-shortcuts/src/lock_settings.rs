//! Typed service and clients for the session-owned Lock Screen policy.

use async_channel::Sender;

use crate::lock::IdlePolicy;

pub const BUS_NAME: &str = "org.rmac.LockScreen1";
pub const OBJECT_PATH: &str = "/org/rmac/LockScreen1";
pub const INTERFACE_NAME: &str = "org.rmac.LockScreen1";
pub type WirePolicy = (u32, u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub lock_after_seconds: Option<u32>,
}

fn encode(policy: IdlePolicy) -> WirePolicy {
    (policy.version, policy.lock_after_seconds.unwrap_or(0))
}

fn decode(policy: WirePolicy) -> Result<IdlePolicy, Error> {
    IdlePolicy {
        version: policy.0,
        lock_after_seconds: (policy.1 != 0).then_some(policy.1),
    }
    .validate()
    .map_err(|_| Error::Invalid)
}

fn snapshot(policy: WirePolicy) -> Result<Snapshot, Error> {
    Ok(Snapshot {
        lock_after_seconds: decode(policy)?.lock_after_seconds,
    })
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.rmac.LockScreen1",
    default_service = "org.rmac.LockScreen1",
    default_path = "/org/rmac/LockScreen1"
)]
trait LockScreen {
    fn settings(&self) -> zbus::Result<WirePolicy>;
    fn set_lock_after(&self, seconds: u32) -> zbus::Result<WirePolicy>;

    #[zbus(signal)]
    fn changed(&self, policy: WirePolicy) -> zbus::Result<()>;
}

#[cfg(target_os = "linux")]
mod service {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard};

    use zbus::connection::Builder;
    use zbus::fdo;
    use zbus::message::Header;
    use zbus::object_server::SignalEmitter;
    use zbus::{interface, Connection};

    use super::{decode, encode, WirePolicy, BUS_NAME, OBJECT_PATH};
    use crate::lock::{self, IdlePolicy};

    #[derive(Clone)]
    struct LockScreenInterface {
        policy: Arc<Mutex<IdlePolicy>>,
        policy_path: PathBuf,
    }

    #[interface(name = "org.rmac.LockScreen1")]
    impl LockScreenInterface {
        fn settings(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WirePolicy> {
            authenticated_sender(&header)?;
            Ok(encode(*lock(&self.policy)?))
        }

        async fn set_lock_after(
            &self,
            seconds: u32,
            #[zbus(header)] header: Header<'_>,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> fdo::Result<WirePolicy> {
            authenticated_sender(&header)?;
            let next = decode((1, seconds))
                .map_err(|_| fdo::Error::InvalidArgs("Lock timeout is invalid".into()))?;
            let policy = self.policy.clone();
            let policy_path = self.policy_path.clone();
            let (wire, changed) = blocking::unblock(move || apply(&policy, &policy_path, next))
                .await
                .map_err(mutation_error)?;
            if changed {
                Self::changed(&emitter, wire).await.map_err(bus_error)?;
            }
            Ok(wire)
        }

        #[zbus(signal)]
        async fn changed(emitter: &SignalEmitter<'_>, policy: WirePolicy) -> zbus::Result<()>;
    }

    pub async fn serve(policy_path: &Path) -> Result<Connection, super::Error> {
        let policy = lock::read_idle_policy(policy_path).map_err(|_| super::Error::Persistence)?;
        let interface = LockScreenInterface {
            policy: Arc::new(Mutex::new(policy)),
            policy_path: policy_path.to_path_buf(),
        };
        Builder::session()
            .map_err(|_| super::Error::Connect)?
            .name(BUS_NAME)
            .map_err(|_| super::Error::Connect)?
            .serve_at(OBJECT_PATH, interface)
            .map_err(|_| super::Error::Connect)?
            .build()
            .await
            .map_err(|_| super::Error::Connect)
    }

    fn lock(policy: &Arc<Mutex<IdlePolicy>>) -> fdo::Result<MutexGuard<'_, IdlePolicy>> {
        policy
            .lock()
            .map_err(|_| fdo::Error::Failed("Lock Screen policy is unavailable".into()))
    }

    fn apply(
        policy: &Arc<Mutex<IdlePolicy>>,
        policy_path: &Path,
        next: IdlePolicy,
    ) -> Result<(WirePolicy, bool), MutationError> {
        let mut current = policy.lock().map_err(|_| MutationError::State)?;
        let before = *current;
        if before == next {
            return Ok((encode(next), false));
        }
        lock::write_idle_policy(policy_path, next).map_err(|_| MutationError::Save)?;
        if lock::restart_idle_manager().is_err() {
            let rollback_saved = lock::write_idle_policy(policy_path, before).is_ok();
            let rollback_started = rollback_saved && lock::restart_idle_manager().is_ok();
            return Err(if rollback_started {
                MutationError::ApplyRestored
            } else {
                MutationError::RollbackFailed
            });
        }
        *current = next;
        Ok((encode(next), true))
    }

    #[derive(Clone, Copy, Debug)]
    enum MutationError {
        State,
        Save,
        ApplyRestored,
        RollbackFailed,
    }

    fn mutation_error(error: MutationError) -> fdo::Error {
        fdo::Error::Failed(
            match error {
                MutationError::State => "Lock Screen policy is unavailable",
                MutationError::Save => "Lock timeout could not be saved",
                MutationError::ApplyRestored => {
                    "Lock timeout could not be applied; the previous timeout was restored"
                }
                MutationError::RollbackFailed => {
                    "Lock timeout failed and its previous state could not be restored"
                }
            }
            .into(),
        )
    }

    fn authenticated_sender(header: &Header<'_>) -> fdo::Result<()> {
        header.sender().map(|_| ()).ok_or_else(|| {
            fdo::Error::AccessDenied("Lock Screen caller identity is unavailable".into())
        })
    }

    fn bus_error(_: zbus::Error) -> fdo::Error {
        fdo::Error::Failed("Lock Screen change could not be published".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use zbus::object_server::Interface;

        #[test]
        fn generated_interface_exposes_settings_mutation_and_signal() {
            let interface = LockScreenInterface {
                policy: Arc::new(Mutex::new(IdlePolicy::default())),
                policy_path: PathBuf::from("/tmp/rmac-lock-policy-test.json"),
            };
            let mut xml = String::new();
            interface.introspect_to_writer(&mut xml, 0);
            assert!(xml.contains("method name=\"Settings\""));
            assert!(xml.contains("method name=\"SetLockAfter\""));
            assert!(xml.contains("signal name=\"Changed\""));
            assert_eq!(super::super::INTERFACE_NAME, BUS_NAME);

            let policy = Arc::new(Mutex::new(IdlePolicy::default()));
            assert_eq!(
                apply(
                    &policy,
                    Path::new("/unused-for-no-op.json"),
                    IdlePolicy::default()
                )
                .unwrap(),
                (encode(IdlePolicy::default()), false)
            );
        }
    }
}

#[cfg(target_os = "linux")]
pub use service::serve;

#[cfg(not(target_os = "linux"))]
pub async fn serve(_policy_path: &std::path::Path) -> Result<(), Error> {
    Err(Error::Connect)
}

#[cfg(target_os = "linux")]
pub fn settings() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = LockScreenProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    snapshot(proxy.settings().map_err(|_| Error::Call)?)
}

#[cfg(not(target_os = "linux"))]
pub fn settings() -> Result<Snapshot, Error> {
    Err(Error::Connect)
}

#[cfg(target_os = "linux")]
pub fn set_lock_after(seconds: Option<u32>) -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = LockScreenProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    snapshot(
        proxy
            .set_lock_after(seconds.unwrap_or(0))
            .map_err(call_error)?,
    )
}

#[cfg(not(target_os = "linux"))]
pub fn set_lock_after(_seconds: Option<u32>) -> Result<Snapshot, Error> {
    Err(Error::Connect)
}

pub async fn watch(sender: Sender<Result<Snapshot, String>>) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        use futures_util::StreamExt as _;

        loop {
            let result = async {
                let connection = zbus::Connection::session()
                    .await
                    .map_err(|_| Error::Connect)?;
                let proxy = LockScreenProxy::new(&connection)
                    .await
                    .map_err(|_| Error::Connect)?;
                let mut changes = proxy
                    .receive_changed()
                    .await
                    .map_err(|_| Error::Subscribe)?;
                publish(
                    &sender,
                    snapshot(proxy.settings().await.map_err(call_error)?)?,
                )
                .await?;
                while changes.next().await.is_some() {
                    publish(
                        &sender,
                        snapshot(proxy.settings().await.map_err(call_error)?)?,
                    )
                    .await?;
                }
                Ok::<(), Error>(())
            }
            .await;
            if sender.is_closed() {
                return Ok(());
            }
            let message = match result {
                Ok(()) => "Lock Screen authority stopped; reconnecting".to_owned(),
                Err(error) => error.to_string(),
            };
            if sender.send(Err(message)).await.is_err() {
                return Ok(());
            }
            let timer = futures_util::FutureExt::fuse(async_io::Timer::after(
                std::time::Duration::from_secs(1),
            ));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(timer, closed);
            futures_util::select! {
                _ = timer => {},
                _ = closed => return Ok(()),
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        sender
            .send(Err(Error::Connect.to_string()))
            .await
            .map_err(|_| Error::Publish)
    }
}

async fn publish(sender: &Sender<Result<Snapshot, String>>, value: Snapshot) -> Result<(), Error> {
    sender.send(Ok(value)).await.map_err(|_| Error::Publish)
}

#[cfg(target_os = "linux")]
fn call_error(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(_, Some(detail), _) if detail.contains("invalid") => {
            Error::Invalid
        }
        zbus::Error::MethodError(_, Some(detail), _)
            if detail.contains("saved") || detail.contains("restored") =>
        {
            Error::Persistence
        }
        _ => Error::Call,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Connect,
    Subscribe,
    Call,
    Invalid,
    Persistence,
    Publish,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "the Lock Screen timeout is invalid",
            Self::Persistence => "the Lock Screen timeout could not be saved or applied",
            Self::Connect | Self::Subscribe | Self::Call | Self::Publish => {
                "the Lock Screen authority is unavailable"
            }
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_policy_round_trips_timeout_and_never() {
        for policy in [
            IdlePolicy::default(),
            IdlePolicy {
                version: 1,
                lock_after_seconds: None,
            },
        ] {
            assert_eq!(decode(encode(policy)).unwrap(), policy);
            assert_eq!(
                snapshot(encode(policy)).unwrap().lock_after_seconds,
                policy.lock_after_seconds
            );
        }
        assert_eq!(decode((2, 300)), Err(Error::Invalid));
        assert_eq!(decode((1, 30)), Err(Error::Invalid));
    }

    #[test]
    fn protocol_identity_is_stable() {
        assert_eq!(BUS_NAME, INTERFACE_NAME);
        assert_eq!(OBJECT_PATH, "/org/rmac/LockScreen1");
    }
}
