//! The session-bus interface, the model worker and the idle exit.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmac_intelligence::{Task, BUS_NAME, IDLE_EXIT_SECONDS, MAX_REQUEST_BYTES, OBJECT_PATH};
use zbus::message::Header;
use zbus::{interface, Connection};

use crate::engine::{self, Engine, EngineError};

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
            EngineError::Failed(_) => Self::Failed(detail),
        }
    }
}

enum JobKind {
    Prepare,
    Intent(String),
    Calibrate,
}

struct Job {
    kind: JobKind,
    received: Instant,
    reply: async_channel::Sender<Result<String, ServiceError>>,
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
            authorize(header, connection).await.inspect_err(|error| {
                eprintln!("rmac-intelligence-service: {error}");
            })?;
            let (reply, answer) = async_channel::bounded(1);
            self.jobs
                .send(Job {
                    kind,
                    received,
                    reply,
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
    /// Replies with `{"intent": <intent>, "timing": {...}}`.
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
        match task {
            Task::Intent => {
                self.submit(JobKind::Intent(text.to_owned()), &header, connection)
                    .await
            }
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

/// Same user, Lulo program (see [`crate::caller`]). One bus round trip:
/// `GetConnectionCredentials` gives the uid, the pid and, on buses that
/// support it, a pidfd. Fails closed; the logged error names the caller
/// and the reason.
#[cfg(target_os = "linux")]
async fn authorize(header: &Header<'_>, connection: &Connection) -> Result<(), ServiceError> {
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
        .map_err(|_| refuse(Refusal::NoCredentials("the bus did not give its credentials")))?;
    crate::caller::check(
        credentials.unix_user_id(),
        credentials.process_id(),
        credentials.process_fd().map(std::os::fd::AsFd::as_fd),
        &crate::caller::trusted_programs(),
    )
    .map(|_| ())
    .map_err(refuse)
}

#[cfg(not(target_os = "linux"))]
async fn authorize(header: &Header<'_>, _connection: &Connection) -> Result<(), ServiceError> {
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

/// The model worker: owns the engine, answers jobs in order. The engine is
/// built on the first job, so an activation that is never used loads
/// nothing.
fn work(jobs: async_channel::Receiver<Job>, state: Arc<Mutex<&'static str>>) {
    let mut engine: Option<Box<dyn Engine>> = None;
    while let Ok(job) = jobs.recv_blocking() {
        let result = (|| -> Result<String, ServiceError> {
            if !engine::enabled() {
                return Err(EngineError::Off.into());
            }
            let cold = engine.is_none();
            if cold {
                set_state(&state, "Loading");
                engine = Some(engine::open().inspect_err(|_| set_state(&state, "Idle"))?);
            }
            let Some(loaded) = engine.as_mut() else {
                return Err(ServiceError::Failed("the model did not load".into()));
            };
            set_state(&state, "Working");
            let reply = match &job.kind {
                JobKind::Prepare => Ok(String::new()),
                JobKind::Intent(text) => loaded.intent(text, job.received).map(|mut outcome| {
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
            };
            set_state(&state, "Ready");
            reply.map_err(ServiceError::from)
        })();
        if let Err(error) = &result {
            // The reason only, never the request.
            eprintln!("rmac-intelligence-service: {error}");
        }
        let _ = job.reply.send_blocking(result);
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
    let worker_state = state.clone();
    std::thread::Builder::new()
        .name("rmac-intelligence-model".into())
        .spawn(move || work(queue, worker_state))
        .map_err(|error| zbus::Error::Failure(error.to_string()))?;
    let connection = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Interface {
                jobs,
                activity: activity_tx,
                state,
            },
        )?
        .build()
        .await?;
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
