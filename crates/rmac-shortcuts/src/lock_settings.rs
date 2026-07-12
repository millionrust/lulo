//! Typed service and clients for the session-owned Lock Screen policy.

use async_channel::Sender;

use crate::lock::IdlePolicy;

pub const BUS_NAME: &str = "org.rmac.LockScreen1";
pub const OBJECT_PATH: &str = "/org/rmac/LockScreen1";
pub const INTERFACE_NAME: &str = "org.rmac.LockScreen1";
pub type WirePolicy = (u32, u32, u32, u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspendCapability {
    Authorized,
    RequiresAuthentication,
    Denied,
    Unavailable,
}

fn decode_suspend_capability(value: &str) -> SuspendCapability {
    match value {
        "yes" => SuspendCapability::Authorized,
        "challenge" => SuspendCapability::RequiresAuthentication,
        "no" => SuspendCapability::Denied,
        _ => SuspendCapability::Unavailable,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub lock_after_seconds: Option<u32>,
    pub suspend_after_seconds: Option<u32>,
    pub suspend_capability: SuspendCapability,
}

fn encode(policy: IdlePolicy, capability: SuspendCapability) -> WirePolicy {
    (
        policy.version,
        policy.lock_after_seconds.unwrap_or(0),
        policy.suspend_after_seconds.unwrap_or(0),
        match capability {
            SuspendCapability::Authorized => 0,
            SuspendCapability::RequiresAuthentication => 1,
            SuspendCapability::Denied => 2,
            SuspendCapability::Unavailable => 3,
        },
    )
}

fn decode(policy: WirePolicy) -> Result<IdlePolicy, Error> {
    IdlePolicy {
        version: policy.0,
        lock_after_seconds: (policy.1 != 0).then_some(policy.1),
        suspend_after_seconds: (policy.2 != 0).then_some(policy.2),
    }
    .validate()
    .map_err(|_| Error::Invalid)
}

fn snapshot(policy: WirePolicy) -> Result<Snapshot, Error> {
    let capability = match policy.3 {
        0 => SuspendCapability::Authorized,
        1 => SuspendCapability::RequiresAuthentication,
        2 => SuspendCapability::Denied,
        3 => SuspendCapability::Unavailable,
        _ => return Err(Error::Protocol),
    };
    let policy = decode(policy)?;
    Ok(Snapshot {
        lock_after_seconds: policy.lock_after_seconds,
        suspend_after_seconds: policy.suspend_after_seconds,
        suspend_capability: capability,
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
    fn set_suspend_after(&self, seconds: u32) -> zbus::Result<WirePolicy>;

    #[zbus(signal)]
    fn changed(&self, policy: WirePolicy) -> zbus::Result<()>;
}

#[cfg(target_os = "linux")]
mod service {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use zbus::connection::Builder;
    use zbus::fdo;
    use zbus::message::Header;
    use zbus::object_server::SignalEmitter;
    use zbus::{interface, Connection};

    use super::{
        decode_suspend_capability, encode, SuspendCapability, WirePolicy, BUS_NAME, OBJECT_PATH,
    };
    use crate::lock::{self, IdlePolicy};

    #[derive(Clone)]
    struct LockScreenInterface {
        policy: Arc<Mutex<IdlePolicy>>,
        policy_path: PathBuf,
    }

    #[interface(name = "org.rmac.LockScreen1")]
    impl LockScreenInterface {
        async fn settings(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WirePolicy> {
            authenticated_sender(&header)?;
            let policy = self.policy.clone();
            blocking::unblock(move || current(&policy))
                .await
                .map_err(mutation_error)
                .map(|(policy, capability)| encode(policy, capability))
        }

        async fn set_lock_after(
            &self,
            seconds: u32,
            #[zbus(header)] header: Header<'_>,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> fdo::Result<WirePolicy> {
            authenticated_sender(&header)?;
            let policy = self.policy.clone();
            let policy_path = self.policy_path.clone();
            let (next, changed, capability) = blocking::unblock(move || {
                apply(
                    &policy,
                    &policy_path,
                    Mutation::Lock((seconds != 0).then_some(seconds)),
                )
            })
            .await
            .map_err(mutation_error)?;
            let wire = encode(next, capability);
            if changed {
                Self::changed(&emitter, wire).await.map_err(bus_error)?;
            }
            Ok(wire)
        }

        async fn set_suspend_after(
            &self,
            seconds: u32,
            #[zbus(header)] header: Header<'_>,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> fdo::Result<WirePolicy> {
            authenticated_sender(&header)?;
            let policy = self.policy.clone();
            let policy_path = self.policy_path.clone();
            let (next, changed, capability) = blocking::unblock(move || {
                apply(
                    &policy,
                    &policy_path,
                    Mutation::Suspend((seconds != 0).then_some(seconds)),
                )
            })
            .await
            .map_err(mutation_error)?;
            let wire = encode(next, capability);
            if changed {
                Self::changed(&emitter, wire).await.map_err(bus_error)?;
            }
            Ok(wire)
        }

        async fn request_suspend(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
            authenticated_sender(&header)?;
            blocking::unblock(perform_suspend)
                .await
                .map_err(mutation_error)
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

    #[zbus::proxy(
        interface = "org.freedesktop.login1.Manager",
        default_service = "org.freedesktop.login1",
        default_path = "/org/freedesktop/login1"
    )]
    trait LoginManager {
        fn can_suspend(&self) -> zbus::Result<String>;
        fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    }

    #[derive(Clone, Copy)]
    enum Mutation {
        Lock(Option<u32>),
        Suspend(Option<u32>),
    }

    fn current(
        policy: &Arc<Mutex<IdlePolicy>>,
    ) -> Result<(IdlePolicy, SuspendCapability), MutationError> {
        let policy = *policy.lock().map_err(|_| MutationError::State)?;
        Ok((policy, suspend_capability()))
    }

    fn apply(
        policy: &Arc<Mutex<IdlePolicy>>,
        policy_path: &Path,
        mutation: Mutation,
    ) -> Result<(IdlePolicy, bool, SuspendCapability), MutationError> {
        let mut current = policy.lock().map_err(|_| MutationError::State)?;
        let before = *current;
        let mut next = before;
        match mutation {
            Mutation::Lock(seconds) => next.lock_after_seconds = seconds,
            Mutation::Suspend(seconds) => next.suspend_after_seconds = seconds,
        }
        next.validate().map_err(|_| MutationError::Invalid)?;
        let capability = suspend_capability();
        if matches!(mutation, Mutation::Suspend(Some(_)))
            && capability != SuspendCapability::Authorized
        {
            return Err(MutationError::AutomaticSuspendUnavailable);
        }
        if before == next {
            return Ok((next, false, capability));
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
        Ok((next, true, capability))
    }

    fn suspend_capability() -> SuspendCapability {
        let Ok(connection) = zbus::blocking::Connection::system() else {
            return SuspendCapability::Unavailable;
        };
        let Ok(manager) = LoginManagerProxyBlocking::new(&connection) else {
            return SuspendCapability::Unavailable;
        };
        manager
            .can_suspend()
            .as_deref()
            .map(decode_suspend_capability)
            .unwrap_or(SuspendCapability::Unavailable)
    }

    fn perform_suspend() -> Result<(), MutationError> {
        let connection =
            zbus::blocking::Connection::system().map_err(|_| MutationError::Capability)?;
        let manager =
            LoginManagerProxyBlocking::new(&connection).map_err(|_| MutationError::Capability)?;
        if manager
            .can_suspend()
            .map_err(|_| MutationError::Capability)?
            != "yes"
        {
            return Err(MutationError::AutomaticSuspendUnavailable);
        }
        manager.suspend(false).map_err(|_| MutationError::Suspend)
    }

    #[derive(Clone, Copy, Debug)]
    enum MutationError {
        State,
        Invalid,
        Capability,
        AutomaticSuspendUnavailable,
        Suspend,
        Save,
        ApplyRestored,
        RollbackFailed,
    }

    fn mutation_error(error: MutationError) -> fdo::Error {
        fdo::Error::Failed(
            match error {
                MutationError::State => "Lock Screen policy is unavailable",
                MutationError::Invalid => "Lock Screen policy is invalid",
                MutationError::Capability => "Suspend capability could not be determined",
                MutationError::AutomaticSuspendUnavailable => {
                    "Automatic suspend is unavailable without authorization"
                }
                MutationError::Suspend => "The automatic suspend request failed",
                MutationError::Save => "Lock Screen policy could not be saved",
                MutationError::ApplyRestored => {
                    "Lock Screen policy could not be applied; the previous policy was restored"
                }
                MutationError::RollbackFailed => {
                    "Lock Screen policy failed and its previous state could not be restored"
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
            assert!(xml.contains("method name=\"SetSuspendAfter\""));
            assert!(xml.contains("method name=\"RequestSuspend\""));
            assert!(xml.contains("signal name=\"Changed\""));
            assert_eq!(super::super::INTERFACE_NAME, BUS_NAME);

            let policy = Arc::new(Mutex::new(IdlePolicy::default()));
            let (applied, changed, _) = apply(
                &policy,
                Path::new("/unused-for-no-op.json"),
                Mutation::Lock(IdlePolicy::default().lock_after_seconds),
            )
            .unwrap();
            assert_eq!(applied, IdlePolicy::default());
            assert!(!changed);
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

#[cfg(target_os = "linux")]
pub fn set_suspend_after(seconds: Option<u32>) -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| Error::Connect)?;
    let proxy = LockScreenProxyBlocking::new(&connection).map_err(|_| Error::Connect)?;
    snapshot(
        proxy
            .set_suspend_after(seconds.unwrap_or(0))
            .map_err(call_error)?,
    )
}

#[cfg(not(target_os = "linux"))]
pub fn set_suspend_after(_seconds: Option<u32>) -> Result<Snapshot, Error> {
    Err(Error::Connect)
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
        zbus::Error::MethodError(_, Some(detail), _)
            if detail.contains("Automatic suspend is unavailable") =>
        {
            Error::AutomaticSuspendUnavailable
        }
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
    AutomaticSuspendUnavailable,
    Persistence,
    Protocol,
    Publish,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "the Lock Screen policy is invalid",
            Self::AutomaticSuspendUnavailable => {
                "automatic suspend is unavailable without authorization"
            }
            Self::Persistence => "the Lock Screen policy could not be saved or applied",
            Self::Connect | Self::Subscribe | Self::Call | Self::Protocol | Self::Publish => {
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
                suspend_after_seconds: None,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(60),
                suspend_after_seconds: Some(30 * 60),
            },
        ] {
            let wire = encode(policy, SuspendCapability::Authorized);
            assert_eq!(decode(wire).unwrap(), policy);
            assert_eq!(
                snapshot(wire).unwrap().lock_after_seconds,
                policy.lock_after_seconds
            );
            assert_eq!(
                snapshot(wire).unwrap().suspend_capability,
                SuspendCapability::Authorized
            );
            assert_eq!(
                snapshot(wire).unwrap().suspend_after_seconds,
                policy.suspend_after_seconds
            );
        }
        assert_eq!(decode((2, 300, 0, 0)), Err(Error::Invalid));
        assert_eq!(decode((1, 30, 0, 0)), Err(Error::Invalid));
        assert_eq!(snapshot((1, 300, 0, 9)), Err(Error::Protocol));
        assert_eq!(
            snapshot(encode(
                IdlePolicy::default(),
                SuspendCapability::RequiresAuthentication
            ))
            .unwrap()
            .suspend_capability,
            SuspendCapability::RequiresAuthentication
        );
    }

    #[test]
    fn protocol_identity_is_stable() {
        assert_eq!(BUS_NAME, INTERFACE_NAME);
        assert_eq!(OBJECT_PATH, "/org/rmac/LockScreen1");
    }

    #[test]
    fn suspend_capabilities_preserve_authorization_semantics() {
        assert_eq!(
            decode_suspend_capability("yes"),
            SuspendCapability::Authorized
        );
        assert_eq!(
            decode_suspend_capability("challenge"),
            SuspendCapability::RequiresAuthentication
        );
        assert_eq!(decode_suspend_capability("no"), SuspendCapability::Denied);
        assert_eq!(
            decode_suspend_capability("na"),
            SuspendCapability::Unavailable
        );
        assert_eq!(
            decode_suspend_capability("future-value"),
            SuspendCapability::Unavailable
        );
    }
}
