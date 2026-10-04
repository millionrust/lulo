use super::*;
use rmac_mail_storage::{Change, NewMessage, FLAG_SEEN};
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

/// A backend that writes one real message into storage on its first sync
/// (as the real `rmac_mail_imap`-backed `ImapBackend` would, whose wire
/// protocol is covered separately by `rmac-mail-imap`'s own fixture-server
/// tests), then does nothing further. Proves MAIL-4's own job: the
/// worker/storage/notification/search pipeline around that protocol layer.
struct FixtureInbox {
    synced: Arc<AtomicUsize>,
}
impl Backend for FixtureInbox {
    fn sync(&mut self, store: &mut MailStorage, account: Uuid) -> Result<Vec<NewMail>, Error> {
        if self.synced.fetch_add(1, Ordering::SeqCst) > 0 {
            return Ok(Vec::new());
        }
        let mailbox_id = store.upsert_mailbox("INBOX", 42, Some("\\Inbox"))?;
        let bytes = b"From: Ada <ada@example.test>\r\nSubject: Lunch on Friday?\r\n\r\nFancy lunch on Friday?";
        let parsed = rmac_mail_mime::parse(bytes)?;
        let id = store.put_message(&NewMessage {
            mailbox_id,
            uid: 1,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject: &parsed.subject,
            sender: "Ada <ada@example.test>",
            recipients: "bob@example.test",
            cc: "",
            preview: &parsed.preview,
            received_at: 1_700_000_000,
            flags: 0,
            body: Some(bytes),
            body_text: Some(&parsed.plain_text),
        })?;
        Ok(vec![NewMail {
            account,
            message_id: id,
            sender: "Ada".into(),
            subject: parsed.subject,
            preview: parsed.preview,
        }])
    }
    fn wait_for_push(&mut self, _: Duration) -> Result<(), Error> {
        panic!("a Graph-transport test account never calls wait_for_push")
    }
}
struct FixtureFactory {
    synced: Arc<AtomicUsize>,
}
impl BackendFactory for FixtureFactory {
    fn connect(&self, _: &Account) -> Result<Box<dyn Backend>, Error> {
        Ok(Box::new(FixtureInbox {
            synced: Arc::clone(&self.synced),
        }))
    }
}
struct CollectingSink {
    snapshots: mpsc::Sender<Snapshot>,
    new_mail: mpsc::Sender<NewMail>,
}
impl EventSink for CollectingSink {
    fn snapshot(&self, value: Snapshot) {
        let _ = self.snapshots.send(value);
    }
    fn new_mail(&self, value: NewMail) {
        let _ = self.new_mail.send(value);
    }
}

/// MAIL-4 end to end at the layer it actually owns: discovering an
/// account, syncing it onto a worker thread, landing the message in
/// `rmac-mail-storage` (readable immediately by FTS5 search), raising a
/// new-mail notification with the unread badge count, marking it read
/// through the same `Change`/journal path a real organise action uses,
/// and the journal entry surviving for the next sync to replay. The wire
/// protocol a real IMAP sync speaks is covered by `rmac-mail-imap`'s own
/// fixture TLS server tests (`tls_plain_sync_move_uidplus_special_use_and_idle`
/// and friends); this proves what sits around it.
#[test]
fn account_discovery_sync_search_and_mark_read_round_trip() {
    let root = std::env::temp_dir().join(format!("mail-runtime-e2e-{}", Uuid::new_v4()));
    let account_id = Uuid::new_v4();
    let (snapshot_tx, snapshot_rx) = mpsc::channel();
    let (new_mail_tx, new_mail_rx) = mpsc::channel();
    let synced = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new(
        root.clone(),
        Arc::new(FixtureFactory {
            synced: Arc::clone(&synced),
        }),
        Arc::new(CollectingSink {
            snapshots: snapshot_tx,
            new_mail: new_mail_tx,
        }),
    );
    // Account discovery (MAIL-4): the GOA-equivalent step for this test is
    // just constructing the `Account` the same way `linux::resolve_goa`
    // would, since GOA itself needs a real session bus.
    runtime.upsert_account(Account {
        path: "/org/gnome/OnlineAccounts/Accounts/1".into(),
        id: account_id,
        address: "ada@example.test".into(),
        transport: Transport::Graph,
    });

    let notice = new_mail_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("a new-mail notification for the synced message");
    assert_eq!(notice.account, account_id);
    assert_eq!(notice.subject, "Lunch on Friday?");

    let snapshot = snapshot_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("a snapshot after the first sync");
    assert_eq!(snapshot.account, account_id);
    assert_eq!(snapshot.unread_inbox, 1, "the unread badge count after sync");
    assert!(snapshot.online);

    // Search (MAIL-7's FTS5 index is populated by the same `put_message`
    // the sync worker just called).
    let mut store = MailStorage::open(&root, account_id).unwrap();
    let hits = store.search("Lunch", None, 10).unwrap();
    assert_eq!(hits.len(), 1, "FTS5 should find the synced message");
    let message_id = hits[0].id;

    // Mark read (the same `Change::SetFlags` a live `MailState::select`'s
    // `Persist` queues): write it to the journal exactly as
    // `crate::live::persist` (the Mail app's own call site) would.
    store
        .queue_change(message_id, Change::SetFlags(FLAG_SEEN))
        .unwrap();
    let pending = store.pending_changes().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending[0].change, Change::SetFlags(bits) if bits == FLAG_SEEN));
    // The flag applies locally immediately, matching the Mac's optimistic
    // UI, ahead of the server round trip a real IMAP worker's `replay()`
    // performs on its next wake.
    assert_eq!(
        store.get_message(message_id).unwrap().unwrap().flags,
        FLAG_SEEN
    );

    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}
