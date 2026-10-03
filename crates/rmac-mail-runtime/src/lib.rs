//! Mail's worker-thread sync coordinator. Construct and drive it off the GPUI
//! thread; its callbacks carry immutable snapshots back to the app.

mod imap;
#[cfg(target_os = "linux")]
pub mod linux;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use rmac_accounts_linux::GoaAccount;
use rmac_mail_storage::MailStorage;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use imap::{CredentialLookup, ImapAuth, ImapFactory, ImapSettings};

pub const IMAP_REFRESH: Duration = Duration::from_secs(15 * 60);
pub const GRAPH_REFRESH: Duration = Duration::from_secs(5 * 60);
const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Imap(ImapSettings),
    Graph,
}

#[derive(Clone)]
pub struct Account {
    pub path: String,
    pub id: Uuid,
    pub address: String,
    pub transport: Transport,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// A stable cache key for a GOA object. The path cannot escape the cache root.
pub fn account_id(account: &GoaAccount) -> Uuid {
    let hash = Sha256::digest(format!("{}:{}", account.provider, account.id).as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    Uuid::from_bytes(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMail {
    pub account: Uuid,
    pub message_id: i64,
    pub sender: String,
    pub subject: String,
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub account: Uuid,
    pub unread_inbox: i64,
    pub online: bool,
}

pub trait EventSink: Send + Sync + 'static {
    fn snapshot(&self, value: Snapshot);
    fn new_mail(&self, value: NewMail);
    fn failure(&self, _account: Uuid, _error: &Error) {}
}

/// MAIL-9 supplies the Graph implementation through this boundary. Graph
/// workers use the same journal and notification path as IMAP workers.
pub trait Backend: Send {
    fn sync(&mut self, store: &mut MailStorage, account: Uuid) -> Result<Vec<NewMail>, Error>;
    /// IMAP waits in IDLE; Graph uses the coordinator's five-minute deadline.
    fn wait_for_push(&mut self, duration: Duration) -> Result<(), Error>;
    fn interrupt(&self) -> Option<rmac_mail_imap::Interrupt> {
        None
    }
}

pub trait BackendFactory: Send + Sync + 'static {
    fn connect(&self, account: &Account) -> Result<Box<dyn Backend>, Error>;
}

#[derive(Debug)]
pub enum Error {
    Account,
    Imap(rmac_mail_imap::Error),
    Storage(rmac_mail_storage::Error),
    Mime(rmac_mail_mime::Error),
    StaleUidValidity,
    GraphUnavailable,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print server text, credentials, addresses, or message data.
        f.write_str(match self {
            Self::Account => "mail account unavailable",
            Self::Imap(_) => "mail server unavailable",
            Self::Storage(_) => "mail cache unavailable",
            Self::Mime(_) => "mail message could not be parsed",
            Self::StaleUidValidity => "mailbox identity changed; local changes need review",
            Self::GraphUnavailable => "Microsoft mail sync unavailable",
        })
    }
}
impl std::error::Error for Error {}
impl From<rmac_mail_imap::Error> for Error {
    fn from(value: rmac_mail_imap::Error) -> Self {
        Self::Imap(value)
    }
}
impl From<rmac_mail_storage::Error> for Error {
    fn from(value: rmac_mail_storage::Error) -> Self {
        Self::Storage(value)
    }
}
impl From<rmac_mail_mime::Error> for Error {
    fn from(value: rmac_mail_mime::Error) -> Self {
        Self::Mime(value)
    }
}

enum Command {
    Online(bool),
    Sync,
    Stop,
}

struct Worker {
    account: Account,
    sender: mpsc::Sender<Command>,
    interrupt: Arc<Mutex<Option<rmac_mail_imap::Interrupt>>>,
}

impl Worker {
    fn send(&self, command: Command) {
        if self.sender.send(command).is_ok() {
            if let Ok(guard) = self.interrupt.lock() {
                if let Some(interrupt) = guard.as_ref() {
                    let _ = interrupt.wake();
                }
            }
        }
    }
}

/// One worker per account. Methods only enqueue commands and never do I/O.
pub struct Runtime {
    workers: Mutex<HashMap<String, Worker>>,
    factory: Arc<dyn BackendFactory>,
    sink: Arc<dyn EventSink>,
    data_root: PathBuf,
    online: Mutex<bool>,
}

impl Runtime {
    pub fn new(
        data_root: PathBuf,
        factory: Arc<dyn BackendFactory>,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            factory,
            sink,
            data_root,
            online: Mutex::new(true),
        }
    }

    pub fn upsert_account(&self, account: Account) {
        let mut workers = self.workers.lock().expect("mail workers lock poisoned");
        if workers.get(&account.path).is_some_and(|worker| {
            worker.account.id == account.id && worker.account.transport == account.transport
        }) {
            return;
        }
        if let Some(old) = workers.remove(&account.path) {
            old.send(Command::Stop);
        }
        let (sender, receiver) = mpsc::channel();
        let interrupt = Arc::new(Mutex::new(None));
        let online = *self.online.lock().expect("mail network lock poisoned");
        let factory = Arc::clone(&self.factory);
        let sink = Arc::clone(&self.sink);
        let root = self.data_root.clone();
        let control = Arc::clone(&interrupt);
        let copy = account.clone();
        thread::spawn(move || run_worker(copy, root, factory, sink, receiver, control, online));
        workers.insert(
            account.path.clone(),
            Worker {
                account,
                sender,
                interrupt,
            },
        );
    }

    pub fn remove_account(&self, path: &str) {
        if let Some(worker) = self
            .workers
            .lock()
            .expect("mail workers lock poisoned")
            .remove(path)
        {
            worker.send(Command::Stop);
        }
    }

    pub fn set_online(&self, online: bool) {
        *self.online.lock().expect("mail network lock poisoned") = online;
        for worker in self
            .workers
            .lock()
            .expect("mail workers lock poisoned")
            .values()
        {
            worker.send(Command::Online(online));
        }
    }

    pub fn sync_now(&self, path: &str) {
        if let Some(worker) = self
            .workers
            .lock()
            .expect("mail workers lock poisoned")
            .get(path)
        {
            worker.send(Command::Sync);
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for (_, worker) in self
            .workers
            .get_mut()
            .expect("mail workers lock poisoned")
            .drain()
        {
            worker.send(Command::Stop);
        }
    }
}

fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_BACKOFF)
}

fn run_worker(
    account: Account,
    root: PathBuf,
    factory: Arc<dyn BackendFactory>,
    sink: Arc<dyn EventSink>,
    receiver: mpsc::Receiver<Command>,
    interrupt: Arc<Mutex<Option<rmac_mail_imap::Interrupt>>>,
    mut online: bool,
) {
    let mut store = match MailStorage::open(&root, account.id) {
        Ok(store) => store,
        Err(error) => {
            sink.failure(account.id, &Error::Storage(error));
            return;
        }
    };
    let mut backoff = Duration::from_secs(1);
    loop {
        if !online {
            sink.snapshot(Snapshot {
                account: account.id,
                unread_inbox: store.unread_inbox_count().unwrap_or(0),
                online: false,
            });
            match receiver.recv() {
                Ok(Command::Online(value)) => online = value,
                Ok(Command::Stop) | Err(_) => break,
                Ok(Command::Sync) => {}
            }
            continue;
        }
        let result = factory.connect(&account).and_then(|mut backend| {
            *interrupt.lock().expect("mail interrupt lock poisoned") = backend.interrupt();
            let result = backend.sync(&mut store, account.id);
            if let Ok(messages) = &result {
                backoff = Duration::from_secs(1);
                for message in messages {
                    sink.new_mail(message.clone());
                }
                sink.snapshot(Snapshot {
                    account: account.id,
                    unread_inbox: store.unread_inbox_count().unwrap_or(0),
                    online: true,
                });
            }
            if result.is_ok() && matches!(&account.transport, Transport::Imap(_)) {
                loop {
                    backend.wait_for_push(IMAP_REFRESH)?;
                    let messages = backend.sync(&mut store, account.id)?;
                    for message in messages {
                        sink.new_mail(message);
                    }
                    sink.snapshot(Snapshot {
                        account: account.id,
                        unread_inbox: store.unread_inbox_count().unwrap_or(0),
                        online: true,
                    });
                }
            }
            result.map(|_| ())
        });
        *interrupt.lock().expect("mail interrupt lock poisoned") = None;
        match receiver.try_recv() {
            Ok(Command::Online(value)) => {
                online = value;
                backoff = Duration::from_secs(1);
                continue;
            }
            Ok(Command::Sync) => {
                backoff = Duration::from_secs(1);
                continue;
            }
            Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Err(error) = &result {
            sink.failure(account.id, error);
        }
        if result.is_ok() && matches!(&account.transport, Transport::Graph) {
            backoff = Duration::from_secs(1);
            match receiver.recv_timeout(GRAPH_REFRESH) {
                Ok(Command::Online(value)) => online = value,
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Command::Sync) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            // A command interrupts IDLE by closing its socket. On a genuine
            // failure, this deadline is the only reconnect wakeup.
            let wait = if result.is_ok() {
                Duration::ZERO
            } else {
                backoff
            };
            match receiver.recv_timeout(wait) {
                Ok(Command::Online(value)) => {
                    online = value;
                    backoff = Duration::from_secs(1);
                }
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Command::Sync) => backoff = Duration::from_secs(1),
                Err(mpsc::RecvTimeoutError::Timeout) => backoff = next_backoff(backoff),
            }
        }
    }
}

#[cfg(test)]
mod tests;
