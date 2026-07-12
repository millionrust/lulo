//! Single-writer Focus authority exported on the user session bus.

use std::sync::{Arc, Mutex, MutexGuard};

use rmac_focus::{ActivationSource, ModeId};
use rmac_focus_runtime::{ClockSampler, PersistenceHealth, Projection, Runtime, Update};
use rmac_notifications::{AppId, BannerPolicy, DeliveryPolicy, HistoryPolicy};
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{interface, Connection};

pub const BUS_NAME: &str = "org.rmac.Focus1";
pub const OBJECT_PATH: &str = "/org/rmac/Focus1";
pub const INTERFACE_NAME: &str = "org.rmac.Focus1";
const DEFAULT_MODE: &str = "do-not-disturb";
pub(crate) const SCHEDULED_DISABLE_DETAIL: &str =
    "scheduled Focus must be changed in Focus settings";

/// Stable D-Bus representation: enabled, mode id, display name, expiry, and
/// whether the latest persistence operation succeeded. Empty strings and a
/// zero expiry represent absent optional values.
pub type WireState = (bool, String, String, u64, bool);
pub type WirePolicy = (bool, bool, bool, u8, bool, bool);

#[derive(Clone)]
struct FocusInterface {
    runtime: Arc<Mutex<Runtime>>,
    clock: Arc<ClockSampler>,
}

#[interface(name = "org.rmac.Focus1")]
impl FocusInterface {
    fn state(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        Ok(wire_state(&*lock(&self.runtime)?))
    }

    fn delivery_policy(
        &self,
        app_id: &str,
        base: WirePolicy,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<WirePolicy> {
        authenticated_sender(&header)?;
        let app_id = AppId::parse(app_id)
            .map_err(|_| fdo::Error::InvalidArgs("notification app id is invalid".into()))?;
        let base = decode_policy(base)
            .map_err(|_| fdo::Error::InvalidArgs("notification policy is invalid".into()))?;
        let policy = lock(&self.runtime)?.enforce(&app_id, base);
        Ok(encode_policy(policy))
    }

    async fn set_enabled(
        &self,
        enabled: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        let before = wire_state(&*lock(&self.runtime)?);
        let (after, scheduled_after) = {
            let mut runtime = lock(&self.runtime)?;
            if enabled {
                if runtime.status().active {
                    (wire_state(&runtime), false)
                } else {
                    let mode = ModeId::parse(DEFAULT_MODE).map_err(domain_error)?;
                    let update = runtime
                        .activate_indefinitely(mode, self.clock.sample())
                        .map_err(runtime_error)?;
                    (wire_update(&runtime, &update), false)
                }
            } else {
                if matches!(runtime.status().source, Some(ActivationSource::Schedule(_))) {
                    return Err(fdo::Error::Failed(SCHEDULED_DISABLE_DETAIL.into()));
                }
                let update = runtime
                    .disable_manual(self.clock.sample())
                    .map_err(runtime_error)?;
                let scheduled = matches!(
                    update.evaluation.status.source,
                    Some(ActivationSource::Schedule(_))
                );
                (wire_update(&runtime, &update), scheduled)
            }
        };
        emit_if_changed(&emitter, &before, &after).await?;
        if scheduled_after {
            return Err(fdo::Error::Failed(SCHEDULED_DISABLE_DETAIL.into()));
        }
        Ok(after)
    }

    async fn activate(
        &self,
        mode_id: &str,
        duration_ms: u64,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        let mode = ModeId::parse(mode_id).map_err(domain_error)?;
        let before = wire_state(&*lock(&self.runtime)?);
        let after = {
            let mut runtime = lock(&self.runtime)?;
            let update = if duration_ms == 0 {
                runtime.activate_indefinitely(mode, self.clock.sample())
            } else {
                runtime.activate_for(mode, duration_ms, self.clock.sample())
            }
            .map_err(runtime_error)?;
            wire_update(&runtime, &update)
        };
        emit_if_changed(&emitter, &before, &after).await?;
        Ok(after)
    }

    async fn disable(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        let before = wire_state(&*lock(&self.runtime)?);
        let (after, scheduled_after) = {
            let mut runtime = lock(&self.runtime)?;
            let update = runtime
                .disable_manual(self.clock.sample())
                .map_err(runtime_error)?;
            let scheduled = matches!(
                update.evaluation.status.source,
                Some(ActivationSource::Schedule(_))
            );
            (wire_update(&runtime, &update), scheduled)
        };
        emit_if_changed(&emitter, &before, &after).await?;
        if scheduled_after {
            return Err(fdo::Error::Failed(SCHEDULED_DISABLE_DETAIL.into()));
        }
        Ok(after)
    }

    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>, state: WireState) -> zbus::Result<()>;
}

pub struct ServiceHandle {
    connection: Connection,
    runtime: Arc<Mutex<Runtime>>,
    clock: Arc<ClockSampler>,
    update: Update,
}

pub async fn serve() -> Result<ServiceHandle, Error> {
    let store = rmac_focus_store::Store::from_environment().map_err(|_| Error::Store)?;
    let clock = Arc::new(ClockSampler::default());
    let (runtime, update) = Runtime::load(store, clock.sample()).map_err(|_| Error::Runtime)?;
    let runtime = Arc::new(Mutex::new(runtime));
    let interface = FocusInterface {
        runtime: runtime.clone(),
        clock: clock.clone(),
    };
    let connection = Builder::session()
        .map_err(|_| Error::Bus)?
        .name(BUS_NAME)
        .map_err(|_| Error::Bus)?
        .serve_at(OBJECT_PATH, interface)
        .map_err(|_| Error::Bus)?
        .build()
        .await
        .map_err(|_| Error::Bus)?;
    Ok(ServiceHandle {
        connection,
        runtime,
        clock,
        update,
    })
}

/// Run exact policy wakeups and resample after Linux time, timezone, and
/// resume hints. The D-Bus connection remains owned by this loop.
pub async fn run(mut handle: ServiceHandle) -> Result<(), Error> {
    let (hint_tx, hint_rx) = async_channel::bounded(8);
    let watcher = crate::watch(hint_tx);
    let evaluator = async {
        loop {
            let now = handle.clock.sample();
            let delay = rmac_focus_runtime::wake_delay(&handle.update, now.unix_ms)
                .unwrap_or(std::time::Duration::from_secs(24 * 60 * 60));
            let timer = futures_util::FutureExt::fuse(async_io::Timer::after(delay));
            let hint = futures_util::FutureExt::fuse(hint_rx.recv());
            futures_util::pin_mut!(timer, hint);
            futures_util::select! {
                _ = timer => {},
                event = hint => match event {
                    Ok(crate::Event::Refresh(_)) | Ok(crate::Event::Unavailable) => {},
                    Err(_) => return Ok(()),
                },
            }
            let (before, after) = {
                let mut runtime = lock(&handle.runtime).map_err(|_| Error::Runtime)?;
                let before = wire_state(&runtime);
                handle.update = runtime
                    .apply_clock(handle.clock.sample())
                    .map_err(|_| Error::Runtime)?;
                let after = wire_update(&runtime, &handle.update);
                (before, after)
            };
            let emitter =
                SignalEmitter::new(&handle.connection, OBJECT_PATH).map_err(|_| Error::Bus)?;
            emit_if_changed(&emitter, &before, &after)
                .await
                .map_err(|_| Error::Bus)?;
        }
    };
    let (_, _) = futures_util::try_join!(
        async { watcher.await.map_err(|_| Error::TimeWatcher) },
        evaluator,
    )?;
    Ok(())
}

fn lock(runtime: &Arc<Mutex<Runtime>>) -> fdo::Result<MutexGuard<'_, Runtime>> {
    runtime
        .lock()
        .map_err(|_| fdo::Error::Failed("Focus runtime state is unavailable".into()))
}

fn wire_state(runtime: &Runtime) -> WireState {
    let status = runtime.status();
    (
        status.active,
        status
            .mode
            .as_ref()
            .map(|mode| mode.as_str().to_owned())
            .unwrap_or_default(),
        status.mode_name.clone().unwrap_or_default(),
        status.ends_at_unix_ms.unwrap_or(0),
        runtime.persistence_health() != PersistenceHealth::SaveFailed,
    )
}

fn wire_update(runtime: &Runtime, update: &Update) -> WireState {
    let mut state = wire_state(runtime);
    state.0 = update.projection.enabled;
    state.2 = update.projection.mode_name.clone().unwrap_or_default();
    state.3 = update.projection.ends_at_unix_ms.unwrap_or(0);
    state
}

pub fn projection(state: &WireState) -> Result<Projection, Error> {
    if state.1.len() > 128
        || state.2.len() > 256
        || (!state.0 && (!state.1.is_empty() || !state.2.is_empty() || state.3 != 0))
        || (state.0 && (state.1.is_empty() || state.2.is_empty()))
    {
        return Err(Error::Protocol);
    }
    Ok(Projection {
        enabled: state.0,
        mode_name: state.0.then(|| state.2.clone()),
        ends_at_unix_ms: (state.3 != 0).then_some(state.3),
    })
}

pub fn encode_policy(policy: DeliveryPolicy) -> WirePolicy {
    (
        policy.enabled,
        policy.banner == BannerPolicy::Allow,
        policy.sounds,
        match policy.history {
            HistoryPolicy::Allow => 0,
            HistoryPolicy::Transient => 1,
            HistoryPolicy::Block => 2,
        },
        policy.allow_urgent_through_focus,
        policy.focus_active,
    )
}

pub fn decode_policy(policy: WirePolicy) -> Result<DeliveryPolicy, Error> {
    Ok(DeliveryPolicy {
        enabled: policy.0,
        banner: if policy.1 {
            BannerPolicy::Allow
        } else {
            BannerPolicy::Suppress
        },
        sounds: policy.2,
        history: match policy.3 {
            0 => HistoryPolicy::Allow,
            1 => HistoryPolicy::Transient,
            2 => HistoryPolicy::Block,
            _ => return Err(Error::Protocol),
        },
        allow_urgent_through_focus: policy.4,
        focus_active: policy.5,
    })
}

async fn emit_if_changed(
    emitter: &SignalEmitter<'_>,
    before: &WireState,
    after: &WireState,
) -> zbus::Result<()> {
    if before != after {
        FocusInterface::changed(emitter, after.clone()).await?;
    }
    Ok(())
}

fn authenticated_sender(header: &Header<'_>) -> fdo::Result<()> {
    header
        .sender()
        .map(|_| ())
        .ok_or_else(|| fdo::Error::AccessDenied("Focus caller identity is unavailable".into()))
}

fn domain_error(_error: rmac_focus::Error) -> fdo::Error {
    fdo::Error::InvalidArgs("Focus mode or duration is invalid".into())
}

fn runtime_error(_error: rmac_focus_runtime::Error) -> fdo::Error {
    fdo::Error::Failed("Focus policy could not be changed".into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Store,
    Runtime,
    Bus,
    TimeWatcher,
    Protocol,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Focus service failed ({self:?})")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::object_server::Interface;

    #[test]
    fn wire_projection_rejects_inconsistent_or_oversized_state() {
        assert!(projection(&(false, String::new(), String::new(), 0, true)).is_ok());
        assert!(projection(&(false, "work".into(), "Work".into(), 0, true)).is_err());
        assert!(projection(&(true, String::new(), "Work".into(), 0, true)).is_err());
        assert!(projection(&(true, "work".into(), "x".repeat(257), 0, true)).is_err());
    }

    #[test]
    fn wire_projection_preserves_live_public_state() {
        let projection = projection(&(true, "work".into(), "Work".into(), 9_000, true)).unwrap();
        assert!(projection.enabled);
        assert_eq!(projection.mode_name.as_deref(), Some("Work"));
        assert_eq!(projection.ends_at_unix_ms, Some(9_000));
    }

    #[test]
    fn delivery_policy_wire_round_trip_preserves_every_capability() {
        let policy = DeliveryPolicy {
            enabled: true,
            banner: BannerPolicy::Suppress,
            sounds: false,
            history: HistoryPolicy::Transient,
            allow_urgent_through_focus: false,
            focus_active: true,
        };
        assert_eq!(decode_policy(encode_policy(policy)).unwrap(), policy);
        assert!(decode_policy((true, true, true, 3, true, false)).is_err());
    }

    #[test]
    fn generated_interface_exposes_state_commands_policy_and_changes() {
        let path = std::env::temp_dir()
            .join(format!("rmac-focus-interface-{}", std::process::id()))
            .join("focus.json");
        let clock = Arc::new(ClockSampler::default());
        let (runtime, _) =
            Runtime::load(rmac_focus_store::Store::at(path), clock.sample()).unwrap();
        let interface = FocusInterface {
            runtime: Arc::new(Mutex::new(runtime)),
            clock,
        };
        let mut xml = String::new();
        interface.introspect_to_writer(&mut xml, 0);
        for method in [
            "State",
            "SetEnabled",
            "Activate",
            "Disable",
            "DeliveryPolicy",
        ] {
            assert!(xml.contains(&format!("method name=\"{method}\"")));
        }
        assert!(xml.contains("signal name=\"Changed\""));
    }
}
