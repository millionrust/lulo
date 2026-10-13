//! `org.rmac.Voice1` at `/org/rmac/Voice1`.
//!
//! - `Start(hands_free)`: open the microphone and start loading the model
//!   (the load overlaps the speech). Only the talk-key HUD may call it
//!   (`caller::Purpose::Voice`), only while Lulo Intelligence and Talk to
//!   Lulo are on and the model is downloaded. One capture at a time.
//! - `Level(d)` about 20 times a second and, in hands-free mode or at the
//!   time limit, one `Ended()`: signals sent to the caller alone.
//! - `Stop() -> s`: close the microphone, transcribe, reply with
//!   `{"text": …, "timing": {…}}`. `Cancel()`: close it, keep nothing.
//! - `EndOnPause()`: the key was tapped after all, so end the capture by
//!   itself after the speech and a pause (then `Ended()`).
//!
//! Audio stays in this process's memory and is dropped with the capture;
//! nothing is written to disk and nothing leaves the PC. The process exits
//! [`IDLE_EXIT_SECONDS`] after its last capture, which unloads the model.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rmac_intelligence::config::Config;
use rmac_intelligence::voice::audio::{meter_level, Endpointer, STEP_SAMPLES};
use rmac_intelligence::voice::{
    self, Availability, VoiceModel, BUS_NAME, IDLE_EXIT_SECONDS, INTERFACE_NAME,
    MAX_CAPTURE_SECONDS, OBJECT_PATH, SAMPLE_RATE,
};
use zbus::message::Header;
use zbus::{interface, Connection};

use super::capture::{self, Source};
use super::engine::{self, Steps, Transcriber, VoiceEngineError};
use crate::caller::Purpose;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.rmac.Voice1.Error")]
pub enum VoiceServiceError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Off(String),
    NotDownloaded(String),
    NoRuntime(String),
    NoMicrophone(String),
    Busy(String),
    Refused(String),
    LowMemory(String),
    Cancelled(String),
    Failed(String),
}

impl From<VoiceEngineError> for VoiceServiceError {
    fn from(error: VoiceEngineError) -> Self {
        let detail = error.to_string();
        match error {
            VoiceEngineError::NoRuntime(_) => Self::NoRuntime(detail),
            VoiceEngineError::NotDownloaded => Self::NotDownloaded(detail),
            VoiceEngineError::Failed(_) => Self::Failed(detail),
        }
    }
}

/// The model, loaded on the first capture and kept until the process
/// exits. `Stop` waits here if the user let go before it finished loading.
#[derive(Default)]
struct EngineSlot {
    state: Mutex<Loaded>,
    changed: Condvar,
}

#[derive(Default)]
enum Loaded {
    #[default]
    Idle,
    Loading,
    Ready {
        engine: Box<dyn Transcriber>,
        load_ms: f64,
    },
    Failed(VoiceEngineError),
}

impl EngineSlot {
    /// Start loading `model` on its own thread unless it is loaded or
    /// loading already.
    fn ensure_loading(self: &Arc<Self>, model: Option<&'static VoiceModel>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if matches!(*state, Loaded::Loading | Loaded::Ready { .. }) {
            return;
        }
        *state = Loaded::Loading;
        drop(state);
        let slot = self.clone();
        let spawned = std::thread::Builder::new()
            .name("rmac-voice-load".into())
            .spawn(move || {
                let started = Instant::now();
                let result = match model {
                    None => {
                        if let Some(delay) = std::env::var("RMAC_VOICE_FIXTURE_LOAD_MS")
                            .ok()
                            .and_then(|value| value.parse::<u64>().ok())
                        {
                            std::thread::sleep(Duration::from_millis(delay.min(10_000)));
                        }
                        Ok(Box::new(engine::FixtureTranscriber::from_environment(
                            capture::capture_file().as_deref(),
                        )) as Box<dyn Transcriber>)
                    }
                    Some(model) => engine::open(model),
                };
                let load_ms = started.elapsed().as_secs_f64() * 1000.0;
                if let Ok(mut state) = slot.state.lock() {
                    *state = match result {
                        Ok(engine) => Loaded::Ready { engine, load_ms },
                        Err(error) => {
                            eprintln!("rmac-voice-service: {error}");
                            Loaded::Failed(error)
                        }
                    };
                }
                slot.changed.notify_all();
            });
        if spawned.is_err() {
            if let Ok(mut state) = self.state.lock() {
                *state = Loaded::Failed(VoiceEngineError::Failed(
                    "the model could not start loading".into(),
                ));
            }
        }
    }

    /// Transcribe once the model is ready. Returns the text, the steps,
    /// how long this waited for the load, and the load's own time.
    fn transcribe(&self, samples: &[f32]) -> Result<(String, Steps, f64, f64), VoiceEngineError> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| VoiceEngineError::Failed("the model worker stopped".into()))?;
        while matches!(*state, Loaded::Loading) {
            state = self
                .changed
                .wait(state)
                .map_err(|_| VoiceEngineError::Failed("the model worker stopped".into()))?;
        }
        let waited = started.elapsed().as_secs_f64() * 1000.0;
        match &mut *state {
            Loaded::Ready { engine, load_ms } => {
                let load_ms = *load_ms;
                let (text, steps) = engine.transcribe(samples)?;
                Ok((text, steps, waited, load_ms))
            }
            Loaded::Failed(error) => Err(error.clone()),
            Loaded::Idle | Loaded::Loading => {
                Err(VoiceEngineError::Failed("the model is not loaded".into()))
            }
        }
    }
}

/// What a finished capture hands to `Stop`.
struct Recorded {
    samples: Vec<f32>,
    start_to_audio_ms: f64,
    error: Option<String>,
}

/// The capture in progress.
struct Active {
    id: u64,
    caller: String,
    stop: Arc<AtomicBool>,
    hands_free: Arc<AtomicBool>,
    done: async_channel::Receiver<Recorded>,
}

/// What the main loop hears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Activity {
    Busy,
    Done,
}

/// Sends a capture's signals to its caller alone.
struct Emitter {
    bus: zbus::blocking::Connection,
    caller: String,
    dbus: Option<zbus::blocking::fdo::DBusProxy<'static>>,
}

impl Emitter {
    fn signal<B>(&self, name: &str, body: &B)
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        let _ = self.bus.emit_signal(
            Some(self.caller.as_str()),
            OBJECT_PATH,
            INTERFACE_NAME,
            name,
            body,
        );
    }

    fn caller_present(&mut self) -> bool {
        if self.dbus.is_none() {
            self.dbus = zbus::blocking::fdo::DBusProxy::new(&self.bus).ok();
        }
        let (Ok(name), Some(dbus)) = (
            zbus::names::BusName::try_from(self.caller.as_str()),
            self.dbus.as_ref(),
        ) else {
            return true;
        };
        dbus.name_has_owner(name).unwrap_or(true)
    }
}

/// The capture thread: read 50 ms steps until told to stop, the caller
/// leaves, the hands-free pause is heard, or the time limit.
fn record(
    mut source: Box<dyn Source>,
    stop: Arc<AtomicBool>,
    hands_free: Arc<AtomicBool>,
    mut emitter: Emitter,
) -> (Recorded, bool) {
    let started = Instant::now();
    let limit = (u64::from(SAMPLE_RATE) * MAX_CAPTURE_SECONDS) as usize;
    let mut samples: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize * 4);
    let mut step: Vec<f32> = Vec::with_capacity(STEP_SAMPLES);
    let mut endpointer = Endpointer::default();
    let mut first_audio: Option<f64> = None;
    let mut error = None;
    let mut ended = false;
    let mut caller_gone = false;
    let mut last_check = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let more = match source.next_step(&mut step) {
            Ok(more) => more,
            Err(read_error) => {
                error = Some(read_error.to_string());
                ended = true;
                break;
            }
        };
        if !step.is_empty() && first_audio.is_none() {
            first_audio = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.extend_from_slice(&step);
        emitter.signal("Level", &(meter_level(&step),));
        if !more {
            if samples.is_empty() {
                error = Some("the microphone delivered no audio".into());
            }
            ended = true;
            break;
        }
        // The detector always follows the audio, so a capture switched to
        // hands-free mid-way knows whether speech already started.
        let paused = endpointer.step(&step);
        if (paused && hands_free.load(Ordering::Relaxed)) || samples.len() >= limit {
            ended = true;
            break;
        }
        if last_check.elapsed() >= Duration::from_secs(1) {
            last_check = Instant::now();
            if !emitter.caller_present() {
                caller_gone = true;
                break;
            }
        }
    }
    source.close();
    if ended && !caller_gone {
        emitter.signal("Ended", &());
    }
    (
        Recorded {
            samples,
            start_to_audio_ms: first_audio.unwrap_or(0.0),
            error,
        },
        caller_gone,
    )
}

struct Interface {
    active: Arc<Mutex<Option<Active>>>,
    next: AtomicU64,
    engine: Arc<EngineSlot>,
    activity: async_channel::Sender<Activity>,
}

fn sender_of(header: &Header<'_>) -> String {
    header
        .sender()
        .map(|sender| sender.to_string())
        .unwrap_or_default()
}

/// Whether the talk key may open the microphone now, and with which model
/// (`None` for the fixture).
fn ready_model() -> Result<Option<&'static VoiceModel>, VoiceServiceError> {
    if engine::fixture_requested() {
        let config = Config::load();
        return if config.enabled && config.voice {
            Ok(None)
        } else {
            Err(VoiceServiceError::Off("Talk to Lulo is turned off".into()))
        };
    }
    match voice::current_availability() {
        Availability::Off | Availability::VoiceOff => {
            Err(VoiceServiceError::Off("Talk to Lulo is turned off".into()))
        }
        Availability::NotOffered(reason) => Err(VoiceServiceError::Off(reason)),
        Availability::NotDownloaded(_) => Err(VoiceServiceError::NotDownloaded(
            "the voice model is not downloaded".into(),
        )),
        Availability::Ready(model) => {
            engine::runtime_present()?;
            let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
            if rmac_intelligence::gate::memory_available_mib(&meminfo)
                .is_some_and(|available| available < model.memory_budget_mib + 256)
            {
                return Err(VoiceServiceError::LowMemory(
                    "not enough free memory right now".into(),
                ));
            }
            Ok(Some(model))
        }
    }
}

#[interface(name = "org.rmac.Voice1")]
impl Interface {
    /// Open the microphone. `hands_free`: the key was tapped, not held, so
    /// the capture ends by itself after speech and a pause.
    async fn start(
        &self,
        hands_free: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<(), VoiceServiceError> {
        authorize(&header, connection)
            .await
            .inspect_err(|error| eprintln!("rmac-voice-service: {error}"))?;
        let model = ready_model()?;
        self.begin(model, sender_of(&header), hands_free, connection)?;
        let _ = self.activity.send(Activity::Busy).await;
        Ok(())
    }

    /// Close the microphone and transcribe what was said.
    async fn stop(&self, #[zbus(header)] header: Header<'_>) -> Result<String, VoiceServiceError> {
        let received = Instant::now();
        let active = self.take_own(&header)?;
        active.stop.store(true, Ordering::Relaxed);
        let result = match active.done.recv().await {
            Err(_) => Err(VoiceServiceError::Cancelled(
                "the capture was abandoned".into(),
            )),
            Ok(recorded) if recorded.samples.is_empty() => Err(VoiceServiceError::NoMicrophone(
                recorded
                    .error
                    .unwrap_or_else(|| "the microphone delivered no audio".into()),
            )),
            Ok(recorded) => self.transcribe(recorded, received).await,
        };
        let _ = self.activity.send(Activity::Done).await;
        result
    }

    /// The key was tapped after all: end by itself after speech and a pause.
    async fn end_on_pause(
        &self,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), VoiceServiceError> {
        let caller = sender_of(&header);
        let active = self
            .active
            .lock()
            .map_err(|_| VoiceServiceError::Failed("the service state is broken".into()))?;
        match active.as_ref() {
            Some(active) if active.caller == caller => {
                active.hands_free.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(VoiceServiceError::Refused(
                "no capture from this caller".into(),
            )),
        }
    }

    /// Close the microphone and keep nothing.
    async fn cancel(&self, #[zbus(header)] header: Header<'_>) -> Result<(), VoiceServiceError> {
        let active = self.take_own(&header)?;
        active.stop.store(true, Ordering::Relaxed);
        let _ = active.done.recv().await;
        let _ = self.activity.send(Activity::Done).await;
        Ok(())
    }
}

impl Interface {
    /// Open the source, start the model loading and the capture thread.
    fn begin(
        &self,
        model: Option<&'static VoiceModel>,
        caller: String,
        hands_free: bool,
        connection: &Connection,
    ) -> Result<(), VoiceServiceError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| VoiceServiceError::Failed("the service state is broken".into()))?;
        if active.is_some() {
            return Err(VoiceServiceError::Busy("Lulo is already listening".into()));
        }
        let source = capture::open().map_err(|error| {
            VoiceServiceError::NoMicrophone(format!("the microphone could not be opened: {error}"))
        })?;
        self.engine.ensure_loading(model);
        let stop = Arc::new(AtomicBool::new(false));
        let hands_free = Arc::new(AtomicBool::new(hands_free));
        let (done_tx, done) = async_channel::bounded(1);
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let emitter = Emitter {
            bus: zbus::blocking::Connection::from(connection.clone()),
            caller: caller.clone(),
            dbus: None,
        };
        let thread_stop = stop.clone();
        let thread_hands_free = hands_free.clone();
        let thread_active = self.active.clone();
        let activity = self.activity.clone();
        std::thread::Builder::new()
            .name("rmac-voice-capture".into())
            .spawn(move || {
                let (recorded, caller_gone) =
                    record(source, thread_stop, thread_hands_free, emitter);
                if caller_gone {
                    // Nobody will call Stop: forget the capture and its audio.
                    if let Ok(mut active) = thread_active.lock() {
                        if active.as_ref().is_some_and(|active| active.id == id) {
                            *active = None;
                        }
                    }
                    let _ = activity.send_blocking(Activity::Done);
                    return;
                }
                let _ = done_tx.send_blocking(recorded);
            })
            .map_err(|_| VoiceServiceError::Failed("the capture could not start".into()))?;
        *active = Some(Active {
            id,
            caller,
            stop,
            hands_free,
            done,
        });
        Ok(())
    }

    /// Transcribe on a worker thread (the bus executor never waits on the
    /// model) and build `Stop`'s reply.
    async fn transcribe(
        &self,
        recorded: Recorded,
        received: Instant,
    ) -> Result<String, VoiceServiceError> {
        let audio_ms = recorded.samples.len() as f64 * 1000.0 / f64::from(SAMPLE_RATE);
        let start_to_audio_ms = recorded.start_to_audio_ms;
        let engine = self.engine.clone();
        let (reply_tx, reply) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("rmac-voice-transcribe".into())
            .spawn(move || {
                let _ = reply_tx.send_blocking(engine.transcribe(&recorded.samples));
            })
            .map_err(|_| VoiceServiceError::Failed("could not transcribe".into()))?;
        match reply.recv().await {
            Ok(Ok((text, steps, waited, load_ms))) => Ok(serde_json::json!({
                "text": voice::clean_transcript(&text),
                "timing": {
                    "audio_ms": audio_ms,
                    "stop_to_text_ms": received.elapsed().as_secs_f64() * 1000.0,
                    "waited_for_model_ms": waited,
                    "preprocess_ms": steps.preprocess_ms,
                    "encode_ms": steps.encode_ms,
                    "decode_ms": steps.decode_ms,
                    "tokens": steps.tokens,
                    "model_load_ms": load_ms,
                    "start_to_audio_ms": start_to_audio_ms,
                },
            })
            .to_string()),
            Ok(Err(error)) => {
                eprintln!("rmac-voice-service: {error}");
                Err(error.into())
            }
            Err(_) => Err(VoiceServiceError::Failed("could not transcribe".into())),
        }
    }

    /// The caller's own capture, taken out of the slot.
    fn take_own(&self, header: &Header<'_>) -> Result<Active, VoiceServiceError> {
        let caller = sender_of(header);
        let mut active = self
            .active
            .lock()
            .map_err(|_| VoiceServiceError::Failed("the service state is broken".into()))?;
        if active.as_ref().is_none_or(|active| active.caller != caller) {
            return Err(VoiceServiceError::Refused(
                "no capture from this caller".into(),
            ));
        }
        active
            .take()
            .ok_or_else(|| VoiceServiceError::Refused("no capture from this caller".into()))
    }
}

/// Same user, and the talk-key HUD (see [`crate::caller`]).
#[cfg(target_os = "linux")]
async fn authorize(header: &Header<'_>, connection: &Connection) -> Result<(), VoiceServiceError> {
    use crate::caller::Refusal;
    let sender = header
        .sender()
        .ok_or_else(|| VoiceServiceError::Refused("the caller is unknown".into()))?
        .to_owned();
    let refuse = |refusal: Refusal| VoiceServiceError::Refused(format!("{sender}: {refusal}"));
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
        &crate::caller::trusted_programs(Purpose::Voice),
    )
    .map(|_| ())
    .map_err(refuse)
}

#[cfg(not(target_os = "linux"))]
async fn authorize(header: &Header<'_>, _connection: &Connection) -> Result<(), VoiceServiceError> {
    let _ = Purpose::Voice;
    header
        .sender()
        .map(|_| ())
        .ok_or_else(|| VoiceServiceError::Refused("the caller is unknown".into()))
}

pub struct ServiceHandle {
    connection: Connection,
    activity: async_channel::Receiver<Activity>,
}

/// Own the bus name.
pub async fn serve() -> zbus::Result<ServiceHandle> {
    let (activity_tx, activity) = async_channel::bounded(64);
    let connection = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Interface {
                active: Arc::new(Mutex::new(None)),
                next: AtomicU64::new(0),
                engine: Arc::new(EngineSlot::default()),
                activity: activity_tx,
            },
        )?
        .build()
        .await?;
    Ok(ServiceHandle {
        connection,
        activity,
    })
}

/// How long the service waits after its last capture before exiting.
/// `RMAC_VOICE_IDLE_SECONDS` shortens it for tests.
pub fn idle_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("RMAC_VOICE_IDLE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| (1..=IDLE_EXIT_SECONDS).contains(seconds))
            .unwrap_or(IDLE_EXIT_SECONDS),
    )
}

/// Serve until idle, then return; the process exits, which frees the model
/// and every buffer. The only wake-up while idle is the one deadline.
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
                Err(_) => break,
            },
        }
    }
    let _ = handle.connection.release_name(BUS_NAME).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_slot_waits_for_its_load() {
        let slot = Arc::new(EngineSlot::default());
        assert!(slot.transcribe(&[0.1; 10]).is_err(), "nothing loaded");
        std::env::set_var("RMAC_VOICE_FIXTURE_TEXT", "open notes");
        std::env::set_var("RMAC_VOICE_FIXTURE_LOAD_MS", "100");
        slot.ensure_loading(None);
        let (text, _, waited, load_ms) = slot.transcribe(&[0.2; 16_000]).unwrap();
        assert_eq!(text, "open notes");
        assert!(waited > 0.0 && load_ms >= 100.0, "{waited} {load_ms}");
        // A second request reuses the loaded engine at once.
        let (_, _, waited, _) = slot.transcribe(&[0.2; 16_000]).unwrap();
        assert!(waited < 50.0);
    }
}
