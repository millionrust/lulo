use super::*;
use std::io::{Read, Write};

struct Fixture {
    root: PathBuf,
    account: Uuid,
    store: MailStorage,
    inbox: i64,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rmac-mail-test-{}", Uuid::new_v4()));
        let account = Uuid::new_v4();
        let mut store = MailStorage::open(&root, account).expect("open fixture");
        let inbox = store
            .upsert_mailbox("INBOX", 42, Some("\\Inbox"))
            .expect("mailbox");
        Self {
            root,
            account,
            store,
            inbox,
        }
    }

    fn insert(&mut self, uid: i64, subject: &str, body: Option<&[u8]>) -> i64 {
        self.store
            .put_message(&NewMessage {
                mailbox_id: self.inbox,
                uid,
                message_id: Some("<example@local>"),
                in_reply_to: None,
                references: &[],
                subject,
                sender: "Ada <ada@example.test>",
                recipients: "Bob <bob@example.test>",
                preview: "A short preview",
                received_at: 100,
                flags: 0,
                body,
                body_text: None,
            })
            .expect("insert")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn sync_cursor_unread_and_pending_local_flags_survive_reopen() {
    let mut fixture = Fixture::new();
    let id = fixture.insert(7, "Unread", None);
    fixture.insert(9, "Second", None);
    assert_eq!(
        fixture.store.cached_uids(fixture.inbox).unwrap(),
        vec![7, 9]
    );
    assert_eq!(fixture.store.unread_inbox_count().unwrap(), 2);
    fixture
        .store
        .set_mailbox_modseq(fixture.inbox, 123)
        .unwrap();
    fixture
        .store
        .queue_change(id, Change::SetFlags(FLAG_SEEN))
        .unwrap();
    fixture.store.set_server_flags(fixture.inbox, 7, 0).unwrap();
    assert_eq!(
        fixture.store.get_message(id).unwrap().unwrap().flags,
        FLAG_SEEN
    );
    assert_eq!(fixture.store.unread_inbox_count().unwrap(), 1);
    let reopened = MailStorage::open(&fixture.root, fixture.account).unwrap();
    assert_eq!(
        reopened.mailbox("INBOX").unwrap().unwrap().highest_modseq,
        123
    );
    assert_eq!(reopened.pending_changes().unwrap().len(), 1);
}

#[test]
fn existing_v2_cache_adds_cursor_without_losing_messages() {
    let mut fixture = Fixture::new();
    fixture.insert(11, "Keep me", None);
    let path = fixture
        .root
        .join(fixture.account.to_string())
        .join("index.sqlite3");
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE mailboxes DROP COLUMN highest_modseq; PRAGMA user_version=2;")
        .unwrap();
    let reopened = MailStorage::open(&fixture.root, fixture.account).unwrap();
    assert_eq!(
        reopened.mailbox("INBOX").unwrap().unwrap().highest_modseq,
        0
    );
    assert_eq!(
        reopened
            .message_by_uid(fixture.inbox, 11)
            .unwrap()
            .unwrap()
            .subject,
        "Keep me"
    );
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .unwrap(),
        3
    );
}

#[test]
fn migration_is_idempotent_and_rejects_future_schema() {
    let fixture = Fixture::new();
    let path = fixture
        .root
        .join(fixture.account.to_string())
        .join("index.sqlite3");
    let reopened = MailStorage::open(&fixture.root, fixture.account).expect("reopen");
    assert_eq!(
        reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .expect("version"),
        3
    );
    drop(reopened);
    let db = Connection::open(path).expect("open raw");
    db.pragma_update(None, "user_version", 999)
        .expect("set version");
    drop(db);
    assert!(matches!(
        MailStorage::open(&fixture.root, fixture.account),
        Err(Error::UnsupportedSchema(999))
    ));
}

#[test]
fn outbox_claim_is_durable_and_crash_safe() {
    let mut fixture = Fixture::new();
    let id = fixture
        .store
        .queue_outbox(
            "a@example.test",
            &["b@example.test".into(), "hidden@example.test".into()],
            b"From: a@example.test\r\n\r\nHello",
        )
        .unwrap();
    assert_eq!(fixture.store.outbox_count(OutboxState::Queued).unwrap(), 1);
    assert_eq!(
        fixture.store.outbox_entries().unwrap()[0].recipients.len(),
        2
    );
    let claimed = fixture.store.claim_outbox().unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.recipients.len(), 2);
    assert!(fixture.store.claim_outbox().unwrap().is_none());
    let mut reopened = MailStorage::open(&fixture.root, fixture.account).unwrap();
    assert_eq!(reopened.outbox_count(OutboxState::Sending).unwrap(), 1);
    assert_eq!(
        reopened.outbox_entries().unwrap()[0].state,
        OutboxState::Sending
    );
    assert!(reopened.claim_outbox().unwrap().is_none());
    reopened.update_outbox_state(id, OutboxState::Held).unwrap();
    assert_eq!(reopened.outbox_count(OutboxState::Held).unwrap(), 1);
    reopened
        .update_outbox_state(id, OutboxState::Queued)
        .unwrap();
    assert_eq!(
        reopened.claim_outbox().unwrap().unwrap().bytes,
        claimed.bytes
    );
    reopened.complete_outbox(id).unwrap();
    assert_eq!(reopened.outbox_count(OutboxState::Queued).unwrap(), 0);
}

#[test]
fn existing_v1_cache_gains_outbox_without_losing_mail() {
    let mut fixture = Fixture::new();
    let message_id = fixture.insert(7, "Before migration", Some(b"retained body"));
    fixture
        .store
        .connection
        .execute_batch("DROP TABLE outbox_recipients; DROP TABLE outbox; ALTER TABLE mailboxes DROP COLUMN highest_modseq; PRAGMA user_version=1;")
        .unwrap();
    let mut reopened = MailStorage::open(&fixture.root, fixture.account).unwrap();
    assert_eq!(
        reopened.get_message(message_id).unwrap().unwrap().subject,
        "Before migration"
    );
    assert_eq!(reopened.outbox_count(OutboxState::Queued).unwrap(), 0);
    reopened
        .queue_outbox("a@example.test", &["b@example.test".into()], b"message")
        .unwrap();
    assert_eq!(reopened.outbox_count(OutboxState::Queued).unwrap(), 1);
    assert_eq!(
        reopened.mailbox("INBOX").unwrap().unwrap().highest_modseq,
        0
    );
}

#[test]
fn blob_is_content_addressed_private_and_detects_damage() {
    let mut fixture = Fixture::new();
    let id = fixture.insert(1, "Hello", Some(b"private body"));
    let hash = fixture
        .store
        .get_message(id)
        .expect("get")
        .expect("message")
        .body_hash
        .expect("hash");
    assert_eq!(
        fixture.store.read_blob(&hash).expect("read"),
        b"private body"
    );
    assert_eq!(
        fixture.store.write_blob(b"private body").expect("dedupe"),
        hash
    );
    assert!(matches!(
        fixture.store.read_blob("../escape"),
        Err(Error::InvalidBlob)
    ));
    let blob = fixture.store.blobs.join(&hash);
    fs::write(&blob, b"tampered").expect("tamper");
    assert!(matches!(
        fixture.store.read_blob(&hash),
        Err(Error::InvalidBlob)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&fixture.store.blobs)
                .expect("dir")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&blob).expect("blob").permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn attachment_metadata_points_to_durable_blob() {
    let mut fixture = Fixture::new();
    let message_id = fixture.insert(1, "Files", None);
    let attachment_id = fixture
        .store
        .put_attachment(message_id, "report.pdf", "application/pdf", b"%PDF fixture")
        .expect("attachment");
    let reopened = MailStorage::open(&fixture.root, fixture.account).expect("reopen");
    let attachments = reopened.attachments(message_id).expect("metadata");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].id, attachment_id);
    assert_eq!(attachments[0].filename, "report.pdf");
    assert_eq!(
        reopened.read_blob(&attachments[0].blob_hash).expect("blob"),
        b"%PDF fixture"
    );
}

#[test]
fn fts_tracks_insert_update_and_delete_and_scopes_mailbox() {
    let mut fixture = Fixture::new();
    let id = fixture.insert(1, "Purple planets", None);
    let other = fixture
        .store
        .upsert_mailbox("Archive", 7, None)
        .expect("archive");
    fixture
        .store
        .put_message(&NewMessage {
            mailbox_id: other,
            uid: 1,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject: "Purple planets",
            sender: "x",
            recipients: "y",
            preview: "z",
            received_at: 1,
            flags: 0,
            body: None,
            body_text: None,
        })
        .expect("other message");
    assert_eq!(
        fixture
            .store
            .search("Purple planets", Some(fixture.inbox), 10)
            .expect("search")
            .len(),
        1
    );
    assert_eq!(
        fixture
            .store
            .search("Purple planets", None, 10)
            .expect("search")
            .len(),
        2
    );
    fixture
        .store
        .put_message(&NewMessage {
            mailbox_id: fixture.inbox,
            uid: 1,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject: "Blue moons",
            sender: "x",
            recipients: "y",
            preview: "z",
            received_at: 2,
            flags: 0,
            body: None,
            body_text: Some("The hidden body speaks"),
        })
        .expect("update");
    assert!(fixture
        .store
        .search("Purple planets", Some(fixture.inbox), 10)
        .expect("search")
        .is_empty());
    assert_eq!(
        fixture
            .store
            .search("hidden body", None, 10)
            .expect("body search")
            .len(),
        1
    );
    assert_eq!(
        fixture
            .store
            .search("Blue moons", Some(fixture.inbox), 10)
            .expect("search")[0]
            .id,
        id
    );
    fixture
        .store
        .connection
        .execute("DELETE FROM messages WHERE id=?1", [id])
        .expect("delete");
    assert!(fixture
        .store
        .search("Blue moons", None, 10)
        .expect("search")
        .is_empty());
    assert!(fixture
        .store
        .search("\" OR private:*", None, 10)
        .expect("escaped search")
        .is_empty());
}

#[test]
fn references_feed_threading_and_survive_reopen() {
    let mut fixture = Fixture::new();
    let parent = fixture.insert(1, "Parent", None);
    fixture
        .store
        .put_message(&NewMessage {
            mailbox_id: fixture.inbox,
            uid: 2,
            message_id: Some("<child>"),
            in_reply_to: Some("<example@local>"),
            references: &["<example@local>"],
            subject: "Other",
            sender: "x",
            recipients: "y",
            preview: "z",
            received_at: 101,
            flags: 0,
            body: None,
            body_text: None,
        })
        .expect("child");
    let reopened = MailStorage::open(&fixture.root, fixture.account).expect("reopen");
    let (ids, forest) = reopened.threads(fixture.inbox).expect("thread");
    assert_eq!(forest.roots.len(), 1);
    assert_eq!(
        ids[forest.nodes[forest.roots[0]].message_index.expect("root")],
        parent
    );
}

#[test]
fn journal_survives_reopen_and_guards_uidvalidity() {
    let mut fixture = Fixture::new();
    let id = fixture.insert(1, "Offline", None);
    let change = fixture
        .store
        .queue_change(id, Change::SetFlags(4))
        .expect("queue");
    assert_eq!(
        fixture
            .store
            .get_message(id)
            .expect("get")
            .expect("message")
            .flags,
        4
    );
    let mut reopened = MailStorage::open(&fixture.root, fixture.account).expect("reopen");
    assert_eq!(reopened.pending_changes().expect("pending").len(), 1);
    assert!(matches!(
        reopened.acknowledge_change(change, 43),
        Err(Error::StaleUidValidity)
    ));
    assert_eq!(reopened.pending_changes().expect("pending").len(), 1);
    reopened.acknowledge_change(change, 42).expect("ack");
    assert!(reopened.pending_changes().expect("pending").is_empty());
}

#[test]
fn killed_writer_rolls_back_uncommitted_row_and_fts() {
    let fixture = Fixture::new();
    let executable = std::env::current_exe().expect("test binary");
    let mut child = std::process::Command::new(executable)
        .arg("--exact")
        .arg("tests::crash_writer_child")
        .arg("--nocapture")
        .env("RMAC_MAIL_CRASH_ROOT", &fixture.root)
        .env("RMAC_MAIL_CRASH_ACCOUNT", fixture.account.to_string())
        .env("RMAC_MAIL_CRASH_MAILBOX", fixture.inbox.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn writer");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut marker = [0u8; 1];
    // The test harness may print its own preamble, so read until the marker.
    loop {
        stdout.read_exact(&mut marker).expect("writer ready");
        if marker[0] == b'!' {
            break;
        }
    }
    child.kill().expect("kill writer");
    child.wait().expect("reap writer");
    let reopened = MailStorage::open(&fixture.root, fixture.account).expect("reopen after crash");
    assert!(reopened
        .search("Uncommitted", None, 10)
        .expect("search")
        .is_empty());
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .expect("count"),
        0
    );
}

#[test]
fn crash_writer_child() {
    let Ok(root) = std::env::var("RMAC_MAIL_CRASH_ROOT") else {
        return;
    };
    let account =
        Uuid::parse_str(&std::env::var("RMAC_MAIL_CRASH_ACCOUNT").expect("account")).expect("uuid");
    let mailbox: i64 = std::env::var("RMAC_MAIL_CRASH_MAILBOX")
        .expect("mailbox")
        .parse()
        .expect("id");
    let mut store = MailStorage::open(Path::new(&root), account).expect("open child");
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("transaction");
    tx.execute("INSERT INTO messages(mailbox_id,uid,subject,sender,recipients,preview,received_at,flags) VALUES (?1,1,'Uncommitted','a','b','c',0,0)", [mailbox]).expect("insert");
    std::io::stdout().write_all(b"!").expect("signal");
    std::io::stdout().flush().expect("flush");
    let mut byte = [0u8; 1];
    std::io::stdin()
        .read_exact(&mut byte)
        .expect("parent holds pipe");
    tx.commit().expect("commit only if test failed to kill");
}
