//! Single-writer Focus authority exported on the user session bus.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use rmac_focus::{ActivationSource, Config, Mode, ModeId, Schedule, ScheduleId, Status, Weekday};
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

/// Stable D-Bus representation: enabled, mode id, display name, expiry,
/// persistence health, activation source, and exact schedule id. Empty strings
/// and zero values represent absent optional values. Source 1 is manual and
/// source 2 is scheduled.
pub type WireState = (bool, String, String, u64, bool, u8, String);
pub type WirePolicy = (bool, bool, bool, u8, bool, bool);
pub type WireMode = (String, String, Vec<String>, bool);
pub type WireSchedule = (String, String, u8, u16, u16, u8, bool);
pub type WireConfiguration = (Vec<WireMode>, Vec<WireSchedule>);
pub type WireSettings = (WireConfiguration, WireState);
const MAX_WIRE_MODES: usize = 32;
const MAX_WIRE_SCHEDULES: usize = 64;
const MAX_WIRE_ALLOWED_APPS: usize = 256;
const EVALUATION_HINT_CAPACITY: usize = 1;

#[derive(Clone)]
struct EvaluationNotifier(async_channel::Sender<()>);

impl EvaluationNotifier {
    /// Coalesce policy mutations into one nonblocking evaluation request. A
    /// closed receiver means the service loop is already terminating; the
    /// mutation itself remains authoritative and must not be reported as if it
    /// had rolled back.
    fn notify(&self) {
        match self.0.try_send(()) {
            Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
            Err(async_channel::TrySendError::Closed(())) => {}
        }
    }
}

#[derive(Clone)]
struct FocusInterface {
    runtime: Arc<Mutex<Runtime>>,
    clock: Arc<ClockSampler>,
    evaluation: EvaluationNotifier,
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

    fn configuration(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireConfiguration> {
        authenticated_sender(&header)?;
        Ok(encode_configuration(lock(&self.runtime)?.config()))
    }

    fn settings(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireSettings> {
        authenticated_sender(&header)?;
        let runtime = lock(&self.runtime)?;
        Ok((encode_configuration(runtime.config()), wire_state(&runtime)))
    }

    async fn replace_configuration(
        &self,
        configuration: WireConfiguration,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        let configuration = decode_configuration(configuration).map_err(configuration_error)?;
        let before = wire_state(&*lock(&self.runtime)?);
        let after = {
            let mut runtime = lock(&self.runtime)?;
            let update = runtime
                .replace_config(configuration, self.clock.sample())
                .map_err(runtime_error)?;
            wire_update(&runtime, &update)
        };
        self.evaluation.notify();
        emit_if_changed(&emitter, &before, &after).await?;
        Self::configuration_changed(&emitter).await?;
        Ok(after)
    }

    async fn set_enabled(
        &self,
        enabled: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<WireState> {
        authenticated_sender(&header)?;
        let before = wire_state(&*lock(&self.runtime)?);
        let (after, scheduled_after, mutated) = {
            let mut runtime = lock(&self.runtime)?;
            if enabled {
                if runtime.status().active {
                    (wire_state(&runtime), false, false)
                } else {
                    let mode = ModeId::parse(DEFAULT_MODE).map_err(domain_error)?;
                    let update = runtime
                        .activate_indefinitely(mode, self.clock.sample())
                        .map_err(runtime_error)?;
                    (wire_update(&runtime, &update), false, true)
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
                (wire_update(&runtime, &update), scheduled, true)
            }
        };
        if mutated {
            self.evaluation.notify();
        }
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
        self.evaluation.notify();
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
        self.evaluation.notify();
        emit_if_changed(&emitter, &before, &after).await?;
        if scheduled_after {
            return Err(fdo::Error::Failed(SCHEDULED_DISABLE_DETAIL.into()));
        }
        Ok(after)
    }

    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>, state: WireState) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn configuration_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub struct ServiceHandle {
    connection: Connection,
    runtime: Arc<Mutex<Runtime>>,
    clock: Arc<ClockSampler>,
    update: Update,
    evaluation_rx: async_channel::Receiver<()>,
}

/// Outgoing calls on this connection (to apps, portals or the bus) give up
/// after this long, so a peer that never replies cannot hold a call open.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn serve() -> Result<ServiceHandle, Error> {
    let store = rmac_focus_store::Store::from_environment().map_err(|_| Error::Store)?;
    let clock = Arc::new(ClockSampler::default());
    let (runtime, update) = Runtime::load(store, clock.sample()).map_err(|_| Error::Runtime)?;
    let runtime = Arc::new(Mutex::new(runtime));
    let (evaluation_tx, evaluation_rx) = async_channel::bounded(EVALUATION_HINT_CAPACITY);
    let interface = FocusInterface {
        runtime: runtime.clone(),
        clock: clock.clone(),
        evaluation: EvaluationNotifier(evaluation_tx),
    };
    let connection = Builder::session()
        .map_err(|_| Error::Bus)?
        .method_timeout(CALL_TIMEOUT)
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
        evaluation_rx,
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
            let delay = rmac_focus_runtime::wake_delay(&handle.update, now.unix_ms);
            let timer = futures_util::FutureExt::fuse(async {
                if let Some(delay) = delay {
                    async_io::Timer::after(delay).await;
                } else {
                    futures_util::future::pending::<()>().await;
                }
            });
            let hint = futures_util::FutureExt::fuse(hint_rx.recv());
            let mutation = futures_util::FutureExt::fuse(handle.evaluation_rx.recv());
            futures_util::pin_mut!(timer, hint, mutation);
            futures_util::select! {
                _ = timer => {},
                event = hint => match event {
                    Ok(crate::Event::Refresh(_)) | Ok(crate::Event::Unavailable) => {},
                    Err(_) => return Ok(()),
                },
                event = mutation => match event {
                    Ok(()) => {},
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
    encode_status(
        runtime.status(),
        runtime.persistence_health() != PersistenceHealth::SaveFailed,
    )
}

fn encode_status(status: &Status, persistence_healthy: bool) -> WireState {
    let (source, schedule_id) = match &status.source {
        None => (0, String::new()),
        Some(ActivationSource::Manual) => (1, String::new()),
        Some(ActivationSource::Schedule(schedule_id)) => (2, schedule_id.as_str().to_owned()),
    };
    (
        status.active,
        status
            .mode
            .as_ref()
            .map(|mode| mode.as_str().to_owned())
            .unwrap_or_default(),
        status.mode_name.clone().unwrap_or_default(),
        status.ends_at_unix_ms.unwrap_or(0),
        persistence_healthy,
        source,
        schedule_id,
    )
}

fn wire_update(runtime: &Runtime, update: &Update) -> WireState {
    encode_status(
        &update.evaluation.status,
        runtime.persistence_health() != PersistenceHealth::SaveFailed,
    )
}

pub fn projection(state: &WireState) -> Result<Projection, Error> {
    if state.1.len() > 128
        || state.2.len() > 256
        || state.2.chars().any(char::is_control)
        || (!state.0 && (!state.1.is_empty() || !state.2.is_empty() || state.3 != 0))
        || (state.0 && (ModeId::parse(&state.1).is_err() || state.2.trim().is_empty()))
    {
        return Err(Error::Protocol);
    }
    activation_source(state)?;
    Ok(Projection {
        enabled: state.0,
        mode_name: state.0.then(|| state.2.clone()),
        ends_at_unix_ms: (state.3 != 0).then_some(state.3),
    })
}

pub fn activation_source(state: &WireState) -> Result<Option<ActivationSource>, Error> {
    match (state.0, state.5, state.6.as_str()) {
        (false, 0, "") => Ok(None),
        (true, 1, "") => Ok(Some(ActivationSource::Manual)),
        (true, 2, schedule_id) => ScheduleId::parse(schedule_id)
            .map(ActivationSource::Schedule)
            .map(Some)
            .map_err(|_| Error::Protocol),
        _ => Err(Error::Protocol),
    }
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

pub fn encode_configuration(configuration: &Config) -> WireConfiguration {
    let modes = configuration
        .modes()
        .map(|mode| {
            (
                mode.id().as_str().to_owned(),
                mode.name().to_owned(),
                mode.allowed_apps()
                    .iter()
                    .map(|app_id| app_id.as_str().to_owned())
                    .collect(),
                mode.allow_urgent(),
            )
        })
        .collect();
    let schedules = configuration
        .schedules()
        .map(|schedule| {
            (
                schedule.id.as_str().to_owned(),
                schedule.mode.as_str().to_owned(),
                weekday_mask(&schedule.days),
                schedule.start_minute,
                schedule.end_minute,
                schedule.priority,
                schedule.enabled,
            )
        })
        .collect();
    (modes, schedules)
}

pub fn decode_configuration(configuration: WireConfiguration) -> Result<Config, Error> {
    if configuration.0.is_empty()
        || configuration.0.len() > MAX_WIRE_MODES
        || configuration.1.len() > MAX_WIRE_SCHEDULES
    {
        return Err(Error::Protocol);
    }
    let modes = configuration
        .0
        .into_iter()
        .map(|(id, name, apps, allow_urgent)| {
            if apps.len() > MAX_WIRE_ALLOWED_APPS {
                return Err(Error::Protocol);
            }
            let app_count = apps.len();
            let allowed_apps = apps
                .into_iter()
                .map(AppId::parse)
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|_| Error::Protocol)?;
            if allowed_apps.len() != app_count {
                return Err(Error::Protocol);
            }
            Mode::new(
                ModeId::parse(id).map_err(|_| Error::Protocol)?,
                name,
                allowed_apps,
                allow_urgent,
            )
            .map_err(|_| Error::Protocol)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let schedules = configuration
        .1
        .into_iter()
        .map(
            |(id, mode, days, start_minute, end_minute, priority, enabled)| {
                Ok(Schedule {
                    id: ScheduleId::parse(id).map_err(|_| Error::Protocol)?,
                    mode: ModeId::parse(mode).map_err(|_| Error::Protocol)?,
                    days: weekdays(days)?,
                    start_minute,
                    end_minute,
                    priority,
                    enabled,
                })
            },
        )
        .collect::<Result<Vec<_>, Error>>()?;
    Config::new(modes, schedules).map_err(|_| Error::Protocol)
}

fn weekday_mask(days: &BTreeSet<Weekday>) -> u8 {
    days.iter().fold(0, |mask, day| mask | weekday_bit(*day))
}

fn weekdays(mask: u8) -> Result<BTreeSet<Weekday>, Error> {
    if mask == 0 || mask & !0b0111_1111 != 0 {
        return Err(Error::Protocol);
    }
    Ok([
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ]
    .into_iter()
    .filter(|day| mask & weekday_bit(*day) != 0)
    .collect())
}

fn weekday_bit(day: Weekday) -> u8 {
    1 << match day {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
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

fn configuration_error(_error: Error) -> fdo::Error {
    fdo::Error::InvalidArgs("Focus configuration is invalid".into())
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
        assert!(projection(&(
            false,
            String::new(),
            String::new(),
            0,
            true,
            0,
            String::new(),
        ))
        .is_ok());
        assert!(projection(&(
            false,
            "work".into(),
            "Work".into(),
            0,
            true,
            0,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            true,
            String::new(),
            "Work".into(),
            0,
            true,
            1,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            true,
            "work".into(),
            "x".repeat(257),
            0,
            true,
            1,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            true,
            "work id".into(),
            "Work".into(),
            0,
            true,
            1,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            true,
            "work".into(),
            "Work\nPrivate".into(),
            0,
            true,
            1,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            true,
            "work".into(),
            "Work".into(),
            0,
            true,
            2,
            String::new(),
        ))
        .is_err());
        assert!(projection(&(
            false,
            String::new(),
            String::new(),
            0,
            true,
            2,
            "weekday".into(),
        ))
        .is_err());
    }

    #[test]
    fn wire_projection_preserves_live_public_state() {
        let projection = projection(&(
            true,
            "work".into(),
            "Work".into(),
            9_000,
            true,
            1,
            String::new(),
        ))
        .unwrap();
        assert!(projection.enabled);
        assert_eq!(projection.mode_name.as_deref(), Some("Work"));
        assert_eq!(projection.ends_at_unix_ms, Some(9_000));
    }

    #[test]
    fn activation_source_round_trips_without_exposing_schedule_in_projection() {
        let manual = (
            true,
            "work".into(),
            "Work".into(),
            0,
            true,
            1,
            String::new(),
        );
        assert_eq!(
            activation_source(&manual).unwrap(),
            Some(ActivationSource::Manual)
        );

        let scheduled = (
            true,
            "work".into(),
            "Work".into(),
            0,
            true,
            2,
            "private-weekday".into(),
        );
        assert_eq!(
            activation_source(&scheduled).unwrap(),
            Some(ActivationSource::Schedule(
                ScheduleId::parse("private-weekday").unwrap()
            ))
        );
        assert!(!format!("{:?}", activation_source(&scheduled)).contains("private-weekday"));
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
    fn configuration_wire_round_trip_is_bounded_and_deterministic() {
        let configuration = rmac_focus_store::default_config().unwrap();
        let wire = encode_configuration(&configuration);
        assert_eq!(decode_configuration(wire.clone()).unwrap(), configuration);
        assert_eq!(encode_configuration(&configuration), wire);

        let mut duplicate_app = wire.clone();
        duplicate_app.0[0].2 = vec!["org.example.App".into(), "org.example.App".into()];
        assert!(decode_configuration(duplicate_app).is_err());

        let mut invalid_days = wire;
        invalid_days.1.push((
            "invalid-days".into(),
            "do-not-disturb".into(),
            0,
            60,
            120,
            1,
            true,
        ));
        assert!(decode_configuration(invalid_days).is_err());
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
            evaluation: EvaluationNotifier(async_channel::bounded(EVALUATION_HINT_CAPACITY).0),
        };
        let mut xml = String::new();
        interface.introspect_to_writer(&mut xml, 0);
        for method in [
            "State",
            "SetEnabled",
            "Activate",
            "Disable",
            "DeliveryPolicy",
            "Configuration",
            "Settings",
            "ReplaceConfiguration",
        ] {
            assert!(xml.contains(&format!("method name=\"{method}\"")));
        }
        assert!(xml.contains("signal name=\"Changed\""));
        assert!(xml.contains("signal name=\"ConfigurationChanged\""));
    }

    #[test]
    fn policy_evaluation_hints_are_bounded_coalesced_and_nonblocking() {
        let (sender, receiver) = async_channel::bounded(EVALUATION_HINT_CAPACITY);
        let notifier = EvaluationNotifier(sender);
        notifier.notify();
        notifier.notify();
        notifier.notify();
        assert_eq!(receiver.len(), 1);
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.is_empty());

        notifier.notify();
        assert_eq!(receiver.try_recv(), Ok(()));
        drop(receiver);
        notifier.notify();
    }
}
