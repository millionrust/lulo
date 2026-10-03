use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FakeGraph {
    syncs: Arc<AtomicUsize>,
    events: mpsc::Sender<usize>,
}
impl Backend for FakeGraph {
    fn sync(&mut self, _store: &mut MailStorage, _account: Uuid) -> Result<Vec<NewMail>, Error> {
        let count = self.syncs.fetch_add(1, Ordering::SeqCst) + 1;
        self.events.send(count).unwrap();
        Ok(Vec::new())
    }
    fn wait_for_push(&mut self, _: Duration) -> Result<(), Error> {
        panic!("Graph must use its deadline")
    }
}
struct FakeFactory {
    syncs: Arc<AtomicUsize>,
    events: mpsc::Sender<usize>,
}
impl BackendFactory for FakeFactory {
    fn connect(&self, _: &Account) -> Result<Box<dyn Backend>, Error> {
        Ok(Box::new(FakeGraph {
            syncs: Arc::clone(&self.syncs),
            events: self.events.clone(),
        }))
    }
}
struct FakeSink;
impl EventSink for FakeSink {
    fn snapshot(&self, _: Snapshot) {}
    fn new_mail(&self, _: NewMail) {}
}

struct TerminalFactory {
    attempts: mpsc::Sender<usize>,
    count: AtomicUsize,
}
impl BackendFactory for TerminalFactory {
    fn connect(&self, _: &Account) -> Result<Box<dyn Backend>, Error> {
        let count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        self.attempts.send(count).unwrap();
        Err(Error::GraphUnavailable)
    }
}

#[test]
fn unavailable_graph_backend_waits_for_an_event_without_polling() {
    let root = std::env::temp_dir().join(format!("mail-runtime-{}", Uuid::new_v4()));
    let (sender, receiver) = mpsc::channel();
    let runtime = Runtime::new(
        root.clone(),
        Arc::new(TerminalFactory {
            attempts: sender,
            count: AtomicUsize::new(0),
        }),
        Arc::new(FakeSink),
    );
    runtime.upsert_account(Account {
        path: "graph".into(),
        id: Uuid::new_v4(),
        address: "test@example.invalid".into(),
        transport: Transport::Graph,
    });
    assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    runtime.sync_now("graph");
    assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn graph_sleeps_until_command_and_reconnects_on_network_event() {
    let root = std::env::temp_dir().join(format!("mail-runtime-{}", Uuid::new_v4()));
    let (sender, receiver) = mpsc::channel();
    let syncs = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new(
        root.clone(),
        Arc::new(FakeFactory {
            syncs: Arc::clone(&syncs),
            events: sender,
        }),
        Arc::new(FakeSink),
    );
    let account = Account {
        path: "fake-goa".into(),
        id: Uuid::new_v4(),
        address: "test@example.invalid".into(),
        transport: Transport::Graph,
    };
    runtime.upsert_account(account);
    assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    runtime.sync_now("fake-goa");
    assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
    runtime.set_online(false);
    assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    runtime.set_online(true);
    assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
    drop(runtime);
    assert_eq!(syncs.load(Ordering::SeqCst), 3);
    // The detached worker exits on Stop; give SQLite's open handle time to close.
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reconnect_backoff_caps_at_five_minutes() {
    let mut delay = Duration::from_secs(1);
    for _ in 0..20 {
        delay = next_backoff(delay);
    }
    assert_eq!(delay, Duration::from_secs(300));
    assert_eq!(IMAP_REFRESH, Duration::from_secs(900));
    assert_eq!(GRAPH_REFRESH, Duration::from_secs(300));
    assert!(Error::GraphUnavailable.needs_user_event());
    assert!(Error::StaleUidValidity.needs_user_event());
    assert!(!Error::Account.needs_user_event());
}

#[test]
fn new_mail_debug_redacts_private_content() {
    let notice = NewMail {
        account: Uuid::new_v4(),
        message_id: 1,
        sender: "planted-sender".into(),
        subject: "planted-subject".into(),
        preview: "planted-preview".into(),
    };
    let printed = format!("{notice:?}");
    assert!(!printed.contains("planted-"));
}

#[test]
fn header_only_messages_still_supply_sender_and_subject() {
    let parsed = rmac_mail_mime::parse(b"From: Ada <ada@example.test>\r\nTo: Bob <bob@example.test>\r\nSubject: First sync\r\n\r\n").unwrap();
    assert_eq!(parsed.subject, "First sync");
    assert_eq!(parsed.from[0].address, "ada@example.test");
    assert!(parsed.preview.is_empty());
}
