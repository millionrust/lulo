//! The session-bus interface, the model worker and the idle exit.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmac_intelligence::client::outcomes;
use rmac_intelligence::{
    writing, Task, BUS_NAME, IDLE_EXIT_SECONDS, INTERFACE_NAME, MAX_REQUEST_BYTES, OBJECT_PATH,
};
use zbus::message::Header;
use zbus::{interface, Connection};

use crate::caller::Purpose;
use crate::engine::{self, Engine, EngineError, WriteRequest};

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.rmac.Intelligence1.Error")]
pub enum ServiceError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Off(String),
    Unavailable(String),
    NotDownloaded(String),
    LowMemory(String),
    Refused(String),
    /// A newer request from the same caller replaced this one.
    Cancelled(String),
    /// The text is longer than the task takes.
    TooLong(String),
    /// The task needs the Standard model.
    NeedsFasterPc(String),
    Failed(String),
}

impl From<EngineError> for ServiceError {
    fn from(error: EngineError) -> Self {
        let detail = error.to_string();
        match error {
            EngineError::Off => Self::Off(detail),
            EngineError::NotSupported(_) => Self::Unavailable(detail),
            EngineError::NotDownloaded => Self::NotDownloaded(detail),
            EngineError::LowMemory => Self::LowMemory(detail),
            EngineError::Cancelled => Self::Cancelled(detail),
            EngineError::TooLong => Self::TooLong(detail),
            EngineError::NeedsFasterPc => Self::NeedsFasterPc(detail),
            EngineError::Failed(_) => Self::Failed(detail),
        }
    }
}

/// `Done`'s outcome for an error.
fn outcome_for(error: &EngineError) -> &'static str {
    match error {
        EngineError::Off => outcomes::OFF,
        EngineError::NotSupported(_) => outcomes::UNAVAILABLE,
        EngineError::NotDownloaded => outcomes::NOT_DOWNLOADED,
        EngineError::LowMemory => outcomes::LOW_MEMORY,
        EngineError::Cancelled => outcomes::CANCELLED,
        EngineError::TooLong => outcomes::TOO_LONG,
        EngineError::NeedsFasterPc => outcomes::NEEDS_FASTER_PC,
        EngineError::Failed(_) => outcomes::FAILED,
    }
}

/// Errors after which nothing will work in this session (turned off, no
/// model): the service exits soon instead of idling for the full minute.
fn exits_soon(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Off
            | EngineError::NotSupported(_)
            | EngineError::NotDownloaded
            | EngineError::LowMemory
    )
}

/// One Writing Tools job, as the worker runs it.
struct WriteJob {
    task: Task,
    text: String,
    attempt: u32,
    id: u64,
    /// The caller's unique bus name: the only destination of its signals.
    caller: String,
    cancel: Arc<AtomicBool>,
}

enum JobKind {
    Prepare,
    Intent {
        text: String,
        /// The caller's unique bus name and this request's ticket, set once
        /// the caller is authorised.
        ticket: Option<(String, u64)>,
    },
    Calibrate,
    Write(WriteJob),
}

/// The newest intent request per caller. Spotlight asks again whenever the
/// query changes, so an older request that is still queued or decoding is
/// stale: the worker drops it before it starts, or at its next forward
/// pass, and the newer one runs at once instead of waiting behind it.
#[derive(Default)]
struct Latest {
    next: AtomicU64,
    by_caller: Mutex<HashMap<String, u64>>,
}

impl Latest {
    fn issue(&self, caller: &str) -> u64 {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut by_caller) = self.by_caller.lock() {
            by_caller.insert(caller.to_owned(), ticket);
        }
        ticket
    }

    fn is_stale(&self, caller: &str, ticket: u64) -> bool {
        self.by_caller.lock().is_ok_and(|by_caller| {
            by_caller
                .get(caller)
                .is_some_and(|newest| *newest != ticket)
        })
    }

    /// Forget a caller once its newest request is answered.
    fn finish(&self, caller: &str, ticket: u64) {
        if let Ok(mut by_caller) = self.by_caller.lock() {
            if by_caller.get(caller) == Some(&ticket) {
                by_caller.remove(caller);
            }
        }
    }
}

/// Writing jobs that have not finished, by job number, with their caller,
/// so `Cancel` stops only the caller's own job.
#[derive(Default)]
struct Running {
    next: AtomicU64,
    jobs: Mutex<HashMap<u64, (String, Arc<AtomicBool>)>>,
}

impl Running {
    fn start(&self, caller: &str) -> (u64, Arc<AtomicBool>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(id, (caller.to_owned(), flag.clone()));
        }
        (id, flag)
    }

    /// Whether `caller` owned `id` (and it is now cancelled).
    fn cancel(&self, caller: &str, id: u64) -> bool {
        self.jobs.lock().is_ok_and(|jobs| {
            jobs.get(&id).is_some_and(|(owner, flag)| {
                let mine = owner == caller;
                if mine {
                    flag.store(true, Ordering::Relaxed);
                }
                mine
            })
        })
    }

    fn finish(&self, id: u64) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&id);
        }
    }
}

struct Job {
    kind: JobKind,
    received: Instant,
    /// `None` for a writing job: its answer goes out as signals.
    reply: Option<async_channel::Sender<Result<String, ServiceError>>>,
}

/// What the main loop hears from the interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Activity {
    /// A call started or finished: the idle deadline moves.
    Busy,
    Done,
    /// Nothing will work in this session (turned off, no model): exit soon
    /// rather than idling for the full minute.
    ExitSoon,
}

struct Interface {
    jobs: async_channel::Sender<Job>,
    activity: async_channel::Sender<Activity>,
    state: Arc<Mutex<&'static str>>,
    latest: Arc<Latest>,
    running: Arc<Running>,
}

fn sender_of(header: &Header<'_>) -> String {
    header
        .sender()
        .map(|sender| sender.to_string())
        .unwrap_or_default()
}

impl Interface {
    async fn submit(
        &self,
        kind: JobKind,
        header: &Header<'_>,
        connection: &Connection,
    ) -> Result<String, ServiceError> {
        let received = Instant::now();
        let _ = self.activity.send(Activity::Busy).await;
        let result = async {
            authorize(header, connection, Purpose::Assist)
                .await
                .inspect_err(|error| {
                    eprintln!("rmac-intelligence-service: {error}");
                })?;
            let kind = match kind {
                JobKind::Intent { text, .. } => {
                    let caller = sender_of(header);
                    let ticket = self.latest.issue(&caller);
                    JobKind::Intent {
                        text,
                        ticket: Some((caller, ticket)),
                    }
                }
                other => other,
            };
            let (reply, answer) = async_channel::bounded(1);
            self.jobs
                .send(Job {
                    kind,
                    received,
                    reply: Some(reply),
                })
                .await
                .map_err(|_| ServiceError::Failed("the model worker stopped".into()))?;
            answer
                .recv()
                .await
                .map_err(|_| ServiceError::Failed("the model worker stopped".into()))?
        }
        .await;
        let next = match &result {
            Err(
                ServiceError::Off(_)
                | ServiceError::Unavailable(_)
                | ServiceError::NotDownloaded(_)
                | ServiceError::LowMemory(_),
            ) => Activity::ExitSoon,
            _ => Activity::Done,
        };
        let _ = self.activity.send(next).await;
        result
    }
}

#[interface(name = "org.rmac.Intelligence1")]
impl Interface {
    /// Load the model and its prompt state now, so a request that follows
    /// is warm.
    async fn prepare(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<(), ServiceError> {
        self.submit(JobKind::Prepare, &header, connection)
            .await
            .map(|_| ())
    }

    /// Run one task from the closed list. `text` is the user's request.
    /// Replies with `{"intent": <intent>, "timing": {...}}`. Writing Tools
    /// tasks use `Write` instead.
    async fn run(
        &self,
        task: &str,
        text: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<String, ServiceError> {
        let Some(task) = Task::parse(task) else {
            return Err(ServiceError::Refused("that task is not on the list".into()));
        };
        if text.len() > MAX_REQUEST_BYTES * 4 {
            return Err(ServiceError::Refused("the request is too long".into()));
        }
        if task.is_writing() {
            return Err(ServiceError::Refused(
                "Writing Tools tasks stream their answer: call Write".into(),
            ));
        }
        self.submit(
            JobKind::Intent {
                text: text.to_owned(),
                ticket: None,
            },
            &header,
            connection,
        )
        .await
    }

    /// Writing Tools: start `task` on `text` and reply at once with a job
    /// number. The answer follows as `Delta(job, text)` signals and one
    /// `Done(job, outcome, detail)`, sent to the caller alone. `attempt` is
    /// 0, then counts up for Try Again.
    async fn write(
        &self,
        task: &str,
        text: &str,
        attempt: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<u64, ServiceError> {
        let Some(task) = Task::parse(task).filter(|task| task.is_writing()) else {
            return Err(ServiceError::Refused(
                "that is not a Writing Tools task".into(),
            ));
        };
        if text.len() > writing::MAX_WRITING_BYTES {
            return Err(ServiceError::TooLong(
                "the text is longer than Writing Tools takes".into(),
            ));
        }
        authorize(&header, connection, Purpose::Writing)
            .await
            .inspect_err(|error| eprintln!("rmac-intelligence-service: {error}"))?;
        let caller = sender_of(&header);
        let (id, cancel) = self.running.start(&caller);
        let _ = self.activity.send(Activity::Busy).await;
        let job = Job {
            kind: JobKind::Write(WriteJob {
                task,
                text: writing::clean_input(text),
                attempt: attempt.min(1000),
                id,
                caller,
                cancel,
            }),
            received: Instant::now(),
            reply: None,
        };
        if self.jobs.send(job).await.is_err() {
            self.running.finish(id);
            let _ = self.activity.send(Activity::Done).await;
            return Err(ServiceError::Failed("the model worker stopped".into()));
        }
        Ok(id)
    }

    /// Stop the caller's own Writing Tools job; its `Done` says "cancelled".
    async fn cancel(
        &self,
        job: u64,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), ServiceError> {
        if self.running.cancel(&sender_of(&header), job) {
            Ok(())
        } else {
            Err(ServiceError::Refused("no such job from this caller".into()))
        }
    }

    /// Measure decode speed for the hardware gate. Replies with
    /// `{"tier": ..., "decode_tok_s": ..., "prefill_tok_s": ...}`.
    async fn calibrate(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<String, ServiceError> {
        self.submit(JobKind::Calibrate, &header, connection).await
    }

    /// `Idle` (nothing loaded), `Loading`, `Ready` or `Working`.
    #[zbus(property)]
    fn state(&self) -> String {
        self.state
            .lock()
            .map(|state| (*state).to_owned())
            .unwrap_or_else(|_| "Idle".to_owned())
    }
}

/// Same user, and a Lulo program listed for `purpose` (see
/// [`crate::caller`]). One bus round trip: `GetConnectionCredentials` gives
/// the uid, the pid and, on buses that support it, a pidfd. Fails closed;
/// the logged error names the caller and the reason.
#[cfg(target_os = "linux")]
async fn authorize(
    header: &Header<'_>,
    connection: &Connection,
    purpose: Purpose,
) -> Result<(), ServiceError> {
    use crate::caller::Refusal;
    let sender = header
        .sender()
        .ok_or_else(|| ServiceError::Refused("the caller is unknown".into()))?
        .to_owned();
    let refuse = |refusal: Refusal| ServiceError::Refused(format!("{sender}: {refusal}"));
    let bus = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|_| refuse(Refusal::NoCredentials("the bus proxy is unavailable")))?;
    let credentials = bus
        .get_connection_credentials(zbus::names::BusName::Unique(sender.clone()))
        .await
        .map_err(|_| {
            refuse(Refusal::NoCredentials(
                "the bus did not give its credentials",
            ))
        })?;
    crate::caller::check(
        credentials.unix_user_id(),
        credentials.process_id(),
        credentials.process_fd().map(std::os::fd::AsFd::as_fd),
        &crate::caller::trusted_programs(purpose),
    )
    .map(|_| ())
    .map_err(refuse)
}

#[cfg(not(target_os = "linux"))]
async fn authorize(
    header: &Header<'_>,
    _connection: &Connection,
    _purpose: Purpose,
) -> Result<(), ServiceError> {
    header
        .sender()
        .map(|_| ())
        .ok_or_else(|| ServiceError::Refused("the caller is unknown".into()))
}

fn set_state(state: &Arc<Mutex<&'static str>>, value: &'static str) {
    if let Ok(mut current) = state.lock() {
        *current = value;
    }
}

/// How often a running writing job checks that its caller is still on the
/// bus: an app that quit mid-answer stops the model within this.
const CALLER_CHECK: Duration = Duration::from_secs(2);

/// What the worker needs besides its queue.
struct Worker {
    state: Arc<Mutex<&'static str>>,
    latest: Arc<Latest>,
    running: Arc<Running>,
    activity: async_channel::Sender<Activity>,
    /// For a writing job's signals.
    bus: zbus::blocking::Connection,
    /// For its caller check, made on the worker thread when first needed.
    dbus: std::cell::OnceCell<Option<zbus::blocking::fdo::DBusProxy<'static>>>,
}

impl Worker {
    fn signal<B>(&self, caller: &str, name: &str, body: &B)
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        let _ = self
            .bus
            .emit_signal(Some(caller), OBJECT_PATH, INTERFACE_NAME, name, body);
    }

    fn caller_present(&self, caller: &str) -> bool {
        let dbus = self
            .dbus
            .get_or_init(|| zbus::blocking::fdo::DBusProxy::new(&self.bus).ok());
        let (Ok(name), Some(dbus)) = (zbus::names::BusName::try_from(caller), dbus) else {
            // A bus that cannot say is not a reason to stop.
            return true;
        };
        dbus.name_has_owner(name).unwrap_or(true)
    }

    /// Load the engine on the first job that needs it.
    fn engine<'a>(
        &self,
        engine: &'a mut Option<Box<dyn Engine>>,
    ) -> Result<(&'a mut Box<dyn Engine>, bool), EngineError> {
        if !engine::enabled() {
            return Err(EngineError::Off);
        }
        let cold = engine.is_none();
        if cold {
            set_state(&self.state, "Loading");
            *engine = Some(engine::open().inspect_err(|_| set_state(&self.state, "Idle"))?);
        }
        let loaded = engine
            .as_mut()
            .ok_or_else(|| EngineError::Failed("the model did not load".into()))?;
        Ok((loaded, cold))
    }

    fn run_write(&self, engine: &mut Option<Box<dyn Engine>>, job: &WriteJob, received: Instant) {
        let last_check = std::cell::Cell::new(Instant::now());
        let cancelled = || {
            if job.cancel.load(Ordering::Relaxed) {
                return true;
            }
            if last_check.get().elapsed() >= CALLER_CHECK {
                last_check.set(Instant::now());
                if !self.caller_present(&job.caller) {
                    job.cancel.store(true, Ordering::Relaxed);
                    return true;
                }
            }
            false
        };
        let result = if cancelled() {
            Err(EngineError::Cancelled)
        } else {
            self.engine(engine).and_then(|(loaded, cold)| {
                set_state(&self.state, "Working");
                let request = WriteRequest {
                    task: job.task,
                    text: &job.text,
                    attempt: job.attempt,
                    received,
                };
                let mut sink = |piece: &str| self.signal(&job.caller, "Delta", &(job.id, piece));
                let outcome = loaded.write(&request, &mut sink, &cancelled);
                set_state(&self.state, "Ready");
                outcome.map(|mut outcome| {
                    outcome.timing.cold = cold;
                    outcome
                })
            })
        };
        let (outcome, detail, next) = match &result {
            Ok(done) => (
                if done.capped {
                    outcomes::CAPPED
                } else {
                    outcomes::FINISHED
                },
                serde_json::json!({"text": done.text, "timing": done.timing}).to_string(),
                Activity::Done,
            ),
            Err(error) => {
                if !matches!(error, EngineError::Cancelled) {
                    // The reason only, never the text.
                    eprintln!("rmac-intelligence-service: writing: {error}");
                }
                (
                    outcome_for(error),
                    serde_json::json!({"error": error.to_string()}).to_string(),
                    if exits_soon(error) {
                        Activity::ExitSoon
                    } else {
                        Activity::Done
                    },
                )
            }
        };
        self.signal(&job.caller, "Done", &(job.id, outcome, detail.as_str()));
        self.running.finish(job.id);
        let _ = self.activity.send_blocking(next);
    }

    /// The model worker: owns the engine, answers jobs in order. The engine
    /// is built on the first job, so an activation that is never used loads
    /// nothing.
    fn work(self, jobs: async_channel::Receiver<Job>) {
        let mut engine: Option<Box<dyn Engine>> = None;
        while let Ok(job) = jobs.recv_blocking() {
            if let JobKind::Write(write) = &job.kind {
                self.run_write(&mut engine, write, job.received);
                continue;
            }
            let ticket = match &job.kind {
                JobKind::Intent { ticket, .. } => ticket.clone(),
                _ => None,
            };
            let stale = || {
                ticket
                    .as_ref()
                    .is_some_and(|(caller, ticket)| self.latest.is_stale(caller, *ticket))
            };
            let result = (|| -> Result<String, ServiceError> {
                if stale() {
                    return Err(EngineError::Cancelled.into());
                }
                let (loaded, cold) = self.engine(&mut engine)?;
                set_state(&self.state, "Working");
                let reply = match &job.kind {
                    JobKind::Prepare => Ok(String::new()),
                    JobKind::Intent { text, .. } => loaded.intent(text, job.received, &stale).map(|mut outcome| {
                        outcome.timing.cold = cold;
                        serde_json::json!({
                            "intent": serde_json::from_str::<serde_json::Value>(&outcome.intent.to_json())
                                .unwrap_or(serde_json::Value::Null),
                            "timing": outcome.timing,
                        })
                        .to_string()
                    }),
                    JobKind::Calibrate => loaded
                        .calibrate()
                        .map(|calibration| serde_json::json!(calibration).to_string()),
                    JobKind::Write(_) => Err(EngineError::Failed("unreachable".into())),
                };
                set_state(&self.state, "Ready");
                reply.map_err(ServiceError::from)
            })();
            match &result {
                Err(ServiceError::Cancelled(_)) | Ok(_) => {}
                // The reason only, never the request.
                Err(error) => eprintln!("rmac-intelligence-service: {error}"),
            }
            if let Some((caller, ticket)) = &ticket {
                self.latest.finish(caller, *ticket);
            }
            if let Some(reply) = job.reply {
                let _ = reply.send_blocking(result);
            }
        }
    }
}

pub struct ServiceHandle {
    connection: Connection,
    activity: async_channel::Receiver<Activity>,
}

/// Own the bus name and start the worker.
pub async fn serve() -> zbus::Result<ServiceHandle> {
    let (jobs, queue) = async_channel::bounded(8);
    let (activity_tx, activity) = async_channel::bounded(64);
    let state = Arc::new(Mutex::new("Idle"));
    let latest = Arc::new(Latest::default());
    let running = Arc::new(Running::default());
    let connection = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Interface {
                jobs,
                activity: activity_tx.clone(),
                state: state.clone(),
                latest: latest.clone(),
                running: running.clone(),
            },
        )?
        .build()
        .await?;
    let worker = Worker {
        state,
        latest,
        running,
        activity: activity_tx,
        bus: zbus::blocking::Connection::from(connection.clone()),
        dbus: std::cell::OnceCell::new(),
    };
    std::thread::Builder::new()
        .name("rmac-intelligence-model".into())
        .spawn(move || worker.work(queue))
        .map_err(|error| zbus::Error::Failure(error.to_string()))?;
    Ok(ServiceHandle {
        connection,
        activity,
    })
}

/// How long the service waits after its last call before exiting.
/// `RMAC_INTELLIGENCE_IDLE_SECONDS` shortens it for tests.
pub fn idle_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("RMAC_INTELLIGENCE_IDLE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| (1..=IDLE_EXIT_SECONDS).contains(seconds))
            .unwrap_or(IDLE_EXIT_SECONDS),
    )
}

/// Serve until idle, then return (the process exits and frees everything).
/// The only wake-up while idle is the one deadline.
pub async fn run(handle: ServiceHandle) {
    let idle = idle_timeout();
    let mut busy = 0usize;
    let mut deadline = Instant::now() + idle;
    loop {
        let timer = futures_util::FutureExt::fuse(async_io::Timer::at(deadline));
        let event = futures_util::FutureExt::fuse(handle.activity.recv());
        futures_util::pin_mut!(timer, event);
        futures_util::select! {
            _ = timer => {
                if busy == 0 {
                    break;
                }
                deadline = Instant::now() + idle;
            }
            event = event => match event {
                Ok(Activity::Busy) => {
                    busy += 1;
                    deadline = Instant::now() + idle;
                }
                Ok(Activity::Done) => {
                    busy = busy.saturating_sub(1);
                    deadline = Instant::now() + idle;
                }
                Ok(Activity::ExitSoon) => {
                    busy = busy.saturating_sub(1);
                    deadline = Instant::now() + Duration::from_secs(1);
                }
                Err(_) => break,
            },
        }
    }
    // Give up the name first, so a request that races the exit activates a
    // fresh service instead of reaching this one.
    let _ = handle.connection.release_name(BUS_NAME).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_newest_request_per_caller_is_current() {
        let latest = Latest::default();
        let first = latest.issue(":1.10");
        let other = latest.issue(":1.11");
        assert!(!latest.is_stale(":1.10", first));
        let second = latest.issue(":1.10");
        assert!(latest.is_stale(":1.10", first));
        assert!(!latest.is_stale(":1.10", second));
        // Another caller's requests are never affected.
        assert!(!latest.is_stale(":1.11", other));
        // Finishing a stale request keeps the newer one current.
        latest.finish(":1.10", first);
        assert!(!latest.is_stale(":1.10", second));
        latest.finish(":1.10", second);
        latest.finish(":1.11", other);
        assert!(latest.by_caller.lock().unwrap().is_empty());
        // An unknown caller is never stale.
        assert!(!latest.is_stale(":1.12", 99));
    }

    #[test]
    fn only_a_jobs_own_caller_can_cancel_it() {
        let running = Running::default();
        let (mine, flag) = running.start(":1.20");
        let (theirs, other_flag) = running.start(":1.21");
        assert_ne!(mine, theirs);
        assert!(!running.cancel(":1.21", mine));
        assert!(!flag.load(Ordering::Relaxed));
        assert!(running.cancel(":1.20", mine));
        assert!(flag.load(Ordering::Relaxed));
        assert!(!other_flag.load(Ordering::Relaxed));
        running.finish(mine);
        assert!(!running.cancel(":1.20", mine), "a finished job is gone");
        running.finish(theirs);
        assert!(running.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn engine_errors_name_their_outcome() {
        assert_eq!(outcome_for(&EngineError::Cancelled), outcomes::CANCELLED);
        assert_eq!(outcome_for(&EngineError::TooLong), outcomes::TOO_LONG);
        assert_eq!(
            outcome_for(&EngineError::NeedsFasterPc),
            outcomes::NEEDS_FASTER_PC
        );
        assert!(exits_soon(&EngineError::Off));
        assert!(!exits_soon(&EngineError::Cancelled));
        assert!(!exits_soon(&EngineError::TooLong));
    }
}
