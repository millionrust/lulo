//! The single-writer clipboard history exported on the user session bus.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rmac_clipboard::{History, WireEntry};
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{interface, Connection};

use crate::store::Store;
use crate::wayland::{self, Change};
use crate::{now_ms, Error, BUS_NAME, OBJECT_PATH};

const EXPIRY_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_RESTART_DELAY: Duration = Duration::from_secs(2);
const CHANGE_CAPACITY: usize = 16;

struct State {
    store: Store,
    history: History,
    enabled: bool,
}

#[derive(Clone)]
struct ClipboardInterface {
    state: Arc<Mutex<State>>,
}

#[interface(name = "org.rmac.Clipboard1")]
impl ClipboardInterface {
    /// Whether the user allowed clipboard history.
    fn enabled(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<bool> {
        authenticated_sender(&header)?;
        Ok(lock(&self.state)?.enabled)
    }

    /// Allow or stop clipboard history. Stopping forgets every item.
    async fn set_enabled(
        &self,
        enabled: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        {
            let mut state = lock(&self.state)?;
            state
                .store
                .set_enabled(enabled)
                .map_err(|_| fdo::Error::Failed("clipboard history choice was not saved".into()))?;
            state.enabled = enabled;
            if !enabled {
                forget_all(&mut state);
            }
        }
        Self::changed(&emitter).await?;
        Ok(())
    }

    /// Newest first. Empty while history is off.
    fn items(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<Vec<WireEntry>> {
        authenticated_sender(&header)?;
        let state = lock(&self.state)?;
        if !state.enabled {
            return Ok(Vec::new());
        }
        Ok(state
            .history
            .entries()
            .iter()
            .map(|entry| rmac_clipboard::encode(entry, &state.store.payload_path(entry.id)))
            .collect())
    }

    /// Put an item back on the clipboard. The watcher then moves it to the
    /// top, as a fresh copy.
    async fn copy(&self, id: u64, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        let (mime, bytes) = {
            let state = lock(&self.state)?;
            if !state.enabled {
                return Err(fdo::Error::Failed(Error::Disabled.to_string()));
            }
            let entry = state
                .history
                .get(id)
                .ok_or_else(|| fdo::Error::InvalidArgs(Error::NotFound.to_string()))?;
            let bytes = state
                .store
                .read_payload(id)
                .map_err(|error| fdo::Error::Failed(error.to_string()))?;
            (entry.mime.clone(), bytes)
        };
        blocking::unblock(move || wayland::write(&mime, &bytes))
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    async fn remove(
        &self,
        id: u64,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        let removed = {
            let mut state = lock(&self.state)?;
            let removed = state.history.remove(id);
            if removed {
                state.store.remove_payloads(&[id]);
                save(&state);
            }
            removed
        };
        if removed {
            Self::changed(&emitter).await?;
        }
        Ok(())
    }

    async fn clear(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        forget_all(&mut *lock(&self.state)?);
        Self::changed(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub struct ServiceHandle {
    connection: Connection,
    state: Arc<Mutex<State>>,
}

pub async fn serve() -> Result<ServiceHandle, Error> {
    let store = Store::from_environment()?;
    store.prepare()?;
    let enabled = store.enabled();
    let mut history = store.load_history();
    if enabled {
        let expired = history.expire(now_ms());
        store.remove_payloads(&expired);
    } else {
        // History left from before the user turned it off is not kept.
        let ids = history.clear();
        store.remove_payloads(&ids);
    }
    let _ = store.save_history(&history);
    let state = Arc::new(Mutex::new(State {
        store,
        history,
        enabled,
    }));
    let connection = Builder::session()
        .map_err(|_| Error::Bus)?
        .name(BUS_NAME)
        .map_err(|_| Error::Bus)?
        .serve_at(
            OBJECT_PATH,
            ClipboardInterface {
                state: state.clone(),
            },
        )
        .map_err(|_| Error::Bus)?
        .build()
        .await
        .map_err(|_| Error::Bus)?;
    Ok(ServiceHandle { connection, state })
}

/// Record clipboard changes and expire old items until the bus closes.
pub async fn run(handle: ServiceHandle) -> Result<(), Error> {
    let (change_tx, change_rx) = async_channel::bounded(CHANGE_CAPACITY);
    // wl-paste blocks its thread; restart it if it exits (compositor
    // restart, wl-clipboard upgrade) for as long as the service runs.
    std::thread::Builder::new()
        .name("rmac-clipboard-watch".into())
        .spawn(move || loop {
            if wayland::watch_blocking(&change_tx).is_ok() || change_tx.is_closed() {
                return;
            }
            std::thread::sleep(WATCH_RESTART_DELAY);
        })
        .map_err(|_| Error::Clipboard)?;

    loop {
        let timer = futures_util::FutureExt::fuse(async_io::Timer::after(EXPIRY_INTERVAL));
        let change = futures_util::FutureExt::fuse(change_rx.recv());
        futures_util::pin_mut!(timer, change);
        let changed = futures_util::select! {
            _ = timer => expire(&handle.state)?,
            change = change => match change {
                Ok(Change::Data) => capture(&handle.state).await?,
                Ok(Change::Sensitive) | Ok(Change::Cleared) => false,
                Err(_) => return Err(Error::Clipboard),
            },
        };
        if changed {
            let emitter =
                SignalEmitter::new(&handle.connection, OBJECT_PATH).map_err(|_| Error::Bus)?;
            ClipboardInterface::changed(&emitter)
                .await
                .map_err(|_| Error::Bus)?;
        }
    }
}

fn expire(state: &Arc<Mutex<State>>) -> Result<bool, Error> {
    let mut state = state.lock().map_err(|_| Error::Store)?;
    let expired = state.history.expire(now_ms());
    if expired.is_empty() {
        return Ok(false);
    }
    state.store.remove_payloads(&expired);
    save(&state);
    Ok(true)
}

/// Read the new selection and record it. Nothing is read while history is
/// off, and nothing but the password-manager hint is read from an offer
/// that carries it with the value `secret`.
async fn capture(state: &Arc<Mutex<State>>) -> Result<bool, Error> {
    if !state.lock().map_err(|_| Error::Store)?.enabled {
        return Ok(false);
    }
    let Some((draft, bytes)) = blocking::unblock(read_selection).await else {
        return Ok(false);
    };
    let mut state = state.lock().map_err(|_| Error::Store)?;
    if !state.enabled {
        return Ok(false);
    }
    let recorded = state.history.record(draft, now_ms());
    if recorded.new_payload && state.store.write_payload(recorded.id, &bytes).is_err() {
        state.history.remove(recorded.id);
        return Ok(false);
    }
    state.store.remove_payloads(&recorded.evicted);
    save(&state);
    Ok(true)
}

fn read_selection() -> Option<(rmac_clipboard::Draft, Vec<u8>)> {
    let types = wayland::offered_types().ok()?;
    if rmac_clipboard::has_sensitivity_hint(&types) {
        let hint = wayland::read(rmac_clipboard::SENSITIVE_HINT_MIME, 64).ok()?;
        // An unreadable or oversized hint is treated as secret.
        if hint.is_none_or(|value| rmac_clipboard::is_secret_hint(&value)) {
            return None;
        }
    }
    for (kind, mime) in rmac_clipboard::candidates(&types) {
        let Ok(Some(bytes)) = wayland::read(&mime, rmac_clipboard::byte_limit(kind)) else {
            continue;
        };
        if let Some(draft) = rmac_clipboard::summarise(kind, &mime, &bytes) {
            return Some((draft, bytes));
        }
    }
    None
}

fn forget_all(state: &mut State) {
    let ids = state.history.clear();
    state.store.remove_payloads(&ids);
    save(state);
}

fn save(state: &State) {
    if state.store.save_history(&state.history).is_err() {
        eprintln!("rmac-clipboard: could not save the clipboard history index");
    }
}

fn lock(state: &Arc<Mutex<State>>) -> fdo::Result<MutexGuard<'_, State>> {
    state
        .lock()
        .map_err(|_| fdo::Error::Failed("clipboard history state is unavailable".into()))
}

fn authenticated_sender(header: &Header<'_>) -> fdo::Result<()> {
    header.sender().map(|_| ()).ok_or_else(|| {
        fdo::Error::AccessDenied("clipboard history caller identity is unavailable".into())
    })
}
