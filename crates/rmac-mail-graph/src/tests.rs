//! Recorded-response tests: every reply is a fixture shaped like Microsoft
//! Graph v1.0's documented JSON. Nothing here opens a socket.

use super::*;
use base64::Engine as _;
use rmac_mail_storage::{OutboxState, FLAG_DRAFT};
use std::sync::Mutex;

const TOKEN: &str = "planted-graph-token";

const BATCH: &[u8] = include_bytes!("../tests/fixtures/batch_well_known.json");
const FOLDERS: &[u8] = include_bytes!("../tests/fixtures/folders.json");
const CHILD_FOLDERS: &[u8] = include_bytes!("../tests/fixtures/child_folders.json");
const INBOX_PAGE1: &[u8] = include_bytes!("../tests/fixtures/inbox_delta_page1.json");
const INBOX_PAGE2: &[u8] = include_bytes!("../tests/fixtures/inbox_delta_page2.json");
const INBOX_CHANGES: &[u8] = include_bytes!("../tests/fixtures/inbox_delta_changes.json");
const INBOX_RESYNC: &[u8] = include_bytes!("../tests/fixtures/inbox_delta_resync.json");
const EMPTY_DELTA: &[u8] = include_bytes!("../tests/fixtures/empty_delta.json");
const MESSAGE_C: &[u8] = include_bytes!("../tests/fixtures/message_c.eml");

const NEXT_PAGE2: &str = "https://graph.microsoft.com/v1.0/me/mailFolders('AAMkInbox%3D')/messages/delta?$skiptoken=page2";
const INBOX_CURSOR_1: &str = "https://graph.microsoft.com/v1.0/me/mailFolders('AAMkInbox%3D')/messages/delta?$deltatoken=inbox1";
const INBOX_CURSOR_2: &str = "https://graph.microsoft.com/v1.0/me/mailFolders('AAMkInbox%3D')/messages/delta?$deltatoken=inbox2";
const INBOX_CURSOR_3: &str = "https://graph.microsoft.com/v1.0/me/mailFolders('AAMkInbox%3D')/messages/delta?$deltatoken=inbox3";
const EMPTY_CURSOR: &str =
    "https://graph.microsoft.com/v1.0/me/mailFolders('other')/messages/delta?$deltatoken=empty";
const OTHER_FOLDERS: [&str; 6] = [
    "AAMkDrafts=",
    "AAMkSent=",
    "AAMkDeleted=",
    "AAMkJunk=",
    "AAMkProjects=",
    "AAMkProjects2026=",
];

/// Canned replies by method and exact URL.
type Routes = HashMap<(Method, String), VecDeque<(u16, Vec<u8>)>>;
/// A non-GET request: method, URL, content type and body.
type Write = (Method, String, Option<&'static str>, Vec<u8>);

struct Recorded {
    method: Method,
    url: String,
    content_type: Option<&'static str>,
    body: Vec<u8>,
    prefer: Option<&'static str>,
}

/// Replays canned replies by method and exact URL. The last reply queued
/// for a route repeats, like an unchanged server.
#[derive(Default)]
struct Fake {
    routes: Mutex<Routes>,
    log: Mutex<Vec<Recorded>>,
}

impl Fake {
    fn on(&self, method: Method, url: &str, status: u16, body: &[u8]) {
        self.routes
            .lock()
            .unwrap()
            .entry((method, url.to_owned()))
            .or_default()
            .push_back((status, body.to_vec()));
    }

    fn replace(&self, method: Method, url: &str, status: u16, body: &[u8]) {
        self.routes.lock().unwrap().insert(
            (method, url.to_owned()),
            VecDeque::from([(status, body.to_vec())]),
        );
    }

    fn writes(&self) -> Vec<Write> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method != Method::Get && !request.url.ends_with("/$batch"))
            .map(|request| {
                (
                    request.method,
                    request.url.clone(),
                    request.content_type,
                    request.body.clone(),
                )
            })
            .collect()
    }

    fn clear_log(&self) {
        self.log.lock().unwrap().clear();
    }
}

impl HttpTransport for Fake {
    fn send(&self, request: &Request, token: &Secret) -> Result<Response, HttpError> {
        assert_eq!(token.expose(), TOKEN, "every request carries GOA's token");
        assert!(
            graph_link(&request.url).is_some(),
            "a request left Microsoft Graph: {}",
            request.url
        );
        self.log.lock().unwrap().push(Recorded {
            method: request.method,
            url: request.url.clone(),
            content_type: request.content_type,
            body: request.body.clone(),
            prefer: request.prefer,
        });
        let mut routes = self.routes.lock().unwrap();
        let queue = routes
            .get_mut(&(request.method, request.url.clone()))
            .unwrap_or_else(|| panic!("unexpected {:?} {}", request.method, request.url));
        let (status, body) = if queue.len() > 1 {
            queue.pop_front().unwrap()
        } else {
            queue.front().cloned().unwrap()
        };
        Ok(Response { status, body })
    }
}

fn url(path: &str) -> String {
    format!("{GRAPH_ROOT}{path}")
}

fn server() -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    fake.on(Method::Post, &url("/$batch"), 200, BATCH);
    fake.on(
        Method::Get,
        &url("/me/mailFolders?$select=id,displayName,childFolderCount&$top=100"),
        200,
        FOLDERS,
    );
    fake.on(
        Method::Get,
        &url("/me/mailFolders/AAMkProjects%3D/childFolders?$select=id,displayName,childFolderCount&$top=100"),
        200,
        CHILD_FOLDERS,
    );
    fake.on(Method::Get, &initial_delta("AAMkInbox="), 200, INBOX_PAGE1);
    fake.on(Method::Get, NEXT_PAGE2, 200, INBOX_PAGE2);
    for id in OTHER_FOLDERS {
        fake.on(Method::Get, &initial_delta(id), 200, EMPTY_DELTA);
    }
    fake.on(Method::Get, EMPTY_CURSOR, 200, EMPTY_DELTA);
    fake
}

struct Harness {
    fake: Arc<Fake>,
    root: PathBuf,
    account: Account,
    store: MailStorage,
    backend: Box<dyn Backend>,
}

impl Harness {
    fn new() -> Self {
        let fake = server();
        let root = std::env::temp_dir().join(format!("rmac-mail-graph-{}", Uuid::new_v4()));
        let transport: Arc<dyn HttpTransport> = fake.clone();
        let factory = GraphFactory::new(
            transport,
            Arc::new(|_: &Account| Ok(Secret::new(TOKEN.to_owned()))),
            root.clone(),
        );
        let account = Account {
            path: "/org/gnome/OnlineAccounts/Accounts/account_1".into(),
            id: Uuid::new_v4(),
            address: "test@example.test".into(),
            transport: Transport::Graph,
        };
        let backend = factory.connect(&account).unwrap();
        let store = MailStorage::open(&root, account.id).unwrap();
        Self {
            fake,
            root,
            account,
            store,
            backend,
        }
    }

    fn sync(&mut self) -> Result<Vec<NewMail>, Error> {
        self.backend.sync(&mut self.store, self.account.id)
    }

    fn message(&self, remote_id: &str) -> Option<rmac_mail_storage::MessageSummary> {
        self.store.message_by_remote_id(remote_id).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn initial_delta_url_selects_only_list_fields_and_encodes_ids() {
    assert_eq!(
        initial_delta("AAMkInbox="),
        "https://graph.microsoft.com/v1.0/me/mailFolders/AAMkInbox%3D/messages/delta?$select=subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,isDraft,flag,bodyPreview,internetMessageId"
    );
    assert_eq!(segment("a+b/c="), "a%2Bb%2Fc%3D");
}

#[test]
fn first_sync_maps_folders_then_delta_brings_new_mail_flags_and_removals() {
    let mut h = Harness::new();
    // First sync: headers only, nothing announced as new.
    assert!(h.sync().unwrap().is_empty());
    let inbox = h.store.mailbox("Inbox").unwrap().unwrap();
    assert_eq!(
        h.store
            .mailbox_by_special_use("\\Inbox")
            .unwrap()
            .unwrap()
            .id,
        inbox.id
    );
    assert_eq!(
        h.store
            .mailbox_by_special_use("\\Trash")
            .unwrap()
            .unwrap()
            .name,
        "Deleted Items"
    );
    assert_eq!(
        h.store
            .mailbox_by_special_use("\\Sent")
            .unwrap()
            .unwrap()
            .name,
        "Sent Items"
    );
    assert!(h
        .store
        .mailbox_by_special_use("\\Archive")
        .unwrap()
        .is_none());
    assert!(h.store.mailbox("Projects/2026").unwrap().is_some());
    let a = h.message("AAMkMsgA=").unwrap();
    assert_eq!(a.mailbox_id, inbox.id);
    assert_eq!(a.sender, "Ada Lovelace <ada@example.test>");
    assert_eq!(a.subject, "Lunch on Friday");
    assert_eq!(a.recipients, "test@example.test");
    assert_eq!(a.flags, 0);
    assert_eq!(
        a.received_at,
        model::unix_time(Some("2026-10-01T09:30:00Z")).unwrap()
    );
    assert!(a.body_hash.is_none());
    let b = h.message("AAMkMsgB=").unwrap();
    assert_eq!(b.flags, FLAG_SEEN | FLAG_FLAGGED);
    assert_eq!(b.cc, "ada@example.test");
    assert_eq!(h.store.unread_inbox_count().unwrap(), 1);
    assert_eq!(
        h.store
            .remote_mailbox("AAMkInbox=")
            .unwrap()
            .unwrap()
            .sync_cursor
            .as_deref(),
        Some(INBOX_CURSOR_1)
    );

    // Second sync resumes from the delta link and fetches the new message.
    h.fake.on(Method::Get, INBOX_CURSOR_1, 200, INBOX_CHANGES);
    h.fake.on(
        Method::Get,
        &url("/me/messages/AAMkMsgC%3D/$value"),
        200,
        MESSAGE_C,
    );
    h.fake.clear_log();
    let arrivals = h.sync().unwrap();
    assert_eq!(arrivals.len(), 1);
    assert_eq!(arrivals[0].subject, "Compiler notes");
    assert_eq!(arrivals[0].sender, "Grace Hopper");
    assert_eq!(arrivals[0].preview, "The notes are attached in spirit.");
    let c = h.message("AAMkMsgC=").unwrap();
    assert_eq!(arrivals[0].message_id, c.id);
    assert!(
        c.body_hash.is_some(),
        "new Inbox mail arrives with its body"
    );
    assert_eq!(c.flags, 0, "downloading a body never marks it read");
    assert_eq!(
        h.message("AAMkMsgA=").unwrap().flags,
        FLAG_SEEN | FLAG_FLAGGED
    );
    assert!(h.message("AAMkMsgB=").is_none(), "@removed drops B");
    assert_eq!(
        h.store
            .remote_mailbox("AAMkInbox=")
            .unwrap()
            .unwrap()
            .sync_cursor
            .as_deref(),
        Some(INBOX_CURSOR_2)
    );
    // A sync only reads: no PATCH, move, delete or send.
    assert!(h.fake.writes().is_empty());
    for request in h.fake.log.lock().unwrap().iter() {
        if request.url.contains("/messages/delta") {
            assert_eq!(request.prefer, Some("odata.maxpagesize=100"));
        }
    }
}

#[test]
fn journal_replays_read_move_delete_and_outbox_sends_mime_with_bcc() {
    let mut h = Harness::new();
    h.sync().unwrap();
    let a = h.message("AAMkMsgA=").unwrap();
    let b = h.message("AAMkMsgB=").unwrap();
    let projects = h.store.mailbox("Projects/2026").unwrap().unwrap().id;
    let drafts = h
        .store
        .mailbox_by_special_use("\\Drafts")
        .unwrap()
        .unwrap()
        .id;
    let local_draft = h
        .store
        .put_message(&NewMessage {
            mailbox_id: drafts,
            uid: 5,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject: "Unsent idea",
            sender: "test@example.test",
            recipients: "",
            cc: "",
            preview: "",
            received_at: 1,
            flags: FLAG_DRAFT,
            body: None,
            body_text: Some("Unsent idea"),
        })
        .unwrap();
    h.store
        .queue_change(a.id, Change::SetFlags(FLAG_SEEN))
        .unwrap();
    h.store.queue_change(b.id, Change::Move(projects)).unwrap();
    h.store.queue_change(local_draft, Change::Delete).unwrap();
    h.store
        .queue_outbox(
            "test@example.test",
            &["to@example.test".into(), "hidden@example.test".into()],
            b"From: test@example.test\r\nTo: to@example.test\r\nSubject: Hi\r\n\r\nHello\r\n",
        )
        .unwrap();
    h.fake.on(
        Method::Patch,
        &url("/me/messages/AAMkMsgA%3D"),
        200,
        b"{\"id\":\"AAMkMsgA=\"}",
    );
    h.fake.on(
        Method::Post,
        &url("/me/messages/AAMkMsgB%3D/move"),
        201,
        b"{\"id\":\"AAMkMsgB=\"}",
    );
    h.fake.on(Method::Post, &url("/me/sendMail"), 202, b"");
    h.fake.on(Method::Get, INBOX_CURSOR_1, 200, EMPTY_DELTA);
    h.fake.clear_log();
    h.sync().unwrap();

    let writes = h.fake.writes();
    assert_eq!(writes.len(), 3, "PATCH, move and sendMail only");
    assert_eq!(writes[0].0, Method::Patch);
    let patch: serde_json::Value = serde_json::from_slice(&writes[0].3).unwrap();
    assert_eq!(
        patch,
        json!({ "isRead": true, "flag": { "flagStatus": "notFlagged" } })
    );
    assert_eq!(writes[1].1, url("/me/messages/AAMkMsgB%3D/move"));
    let moved: serde_json::Value = serde_json::from_slice(&writes[1].3).unwrap();
    assert_eq!(moved, json!({ "destinationId": "AAMkProjects2026=" }));
    assert_eq!(writes[2].1, url("/me/sendMail"));
    assert_eq!(writes[2].2, Some("text/plain"));
    let mime = base64::engine::general_purpose::STANDARD
        .decode(&writes[2].3)
        .unwrap();
    let mime = String::from_utf8(mime).unwrap();
    assert!(mime.starts_with("Bcc: hidden@example.test\r\n"));
    assert!(mime.contains("To: to@example.test\r\n"));

    assert!(h.store.pending_changes().unwrap().is_empty());
    assert!(h.store.outbox_entries().unwrap().is_empty());
    assert!(h.message("AAMkMsgB=").is_none(), "B left the Inbox");
    assert!(h.store.message_by_uid(drafts, 5).unwrap().is_none());
    assert_eq!(h.message("AAMkMsgA=").unwrap().flags & FLAG_SEEN, FLAG_SEEN);

    // Delete outside Deleted Items moves there; inside it deletes for good.
    let a = h.message("AAMkMsgA=").unwrap();
    h.store.queue_change(a.id, Change::Delete).unwrap();
    let trash = h
        .store
        .mailbox_by_special_use("\\Trash")
        .unwrap()
        .unwrap()
        .id;
    let old = h
        .store
        .put_message(&NewMessage {
            mailbox_id: trash,
            uid: 9,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject: "Old",
            sender: "x@example.test",
            recipients: "",
            cc: "",
            preview: "",
            received_at: 1,
            flags: FLAG_SEEN,
            body: None,
            body_text: None,
        })
        .unwrap();
    h.store.bind_remote_message(old, "AAMkOld=").unwrap();
    h.store.queue_change(old, Change::Delete).unwrap();
    h.fake.on(
        Method::Post,
        &url("/me/messages/AAMkMsgA%3D/move"),
        201,
        b"{\"id\":\"AAMkMsgA=\"}",
    );
    h.fake
        .on(Method::Delete, &url("/me/messages/AAMkOld%3D"), 204, b"");
    h.fake.on(Method::Get, EMPTY_CURSOR, 200, EMPTY_DELTA);
    h.fake.clear_log();
    h.sync().unwrap();
    let writes = h.fake.writes();
    assert_eq!(writes.len(), 2);
    let to_trash: serde_json::Value = serde_json::from_slice(&writes[0].3).unwrap();
    assert_eq!(to_trash, json!({ "destinationId": "deleteditems" }));
    assert_eq!(
        (writes[1].0, writes[1].1.as_str()),
        (Method::Delete, url("/me/messages/AAMkOld%3D").as_str())
    );
    assert!(h.message("AAMkOld=").is_none());
    assert!(h.store.pending_changes().unwrap().is_empty());
}

#[test]
fn a_change_to_a_message_already_gone_on_the_server_is_dropped() {
    let mut h = Harness::new();
    h.sync().unwrap();
    let a = h.message("AAMkMsgA=").unwrap();
    h.store
        .queue_change(a.id, Change::SetFlags(FLAG_FLAGGED))
        .unwrap();
    h.fake.on(
        Method::Patch,
        &url("/me/messages/AAMkMsgA%3D"),
        404,
        br#"{"error":{"code":"ErrorItemNotFound","message":"The specified object was not found in the store."}}"#,
    );
    h.fake.on(Method::Get, INBOX_CURSOR_1, 200, EMPTY_DELTA);
    h.sync().unwrap();
    assert!(h.store.pending_changes().unwrap().is_empty());
}

#[test]
fn expired_delta_cursor_resyncs_the_folder_and_reconciles() {
    let mut h = Harness::new();
    h.sync().unwrap();
    assert!(h.message("AAMkMsgB=").is_some());
    h.fake.on(
        Method::Get,
        INBOX_CURSOR_1,
        410,
        br#"{"error":{"code":"SyncStateNotFound","message":"The sync state generation is not found."}}"#,
    );
    h.fake
        .replace(Method::Get, &initial_delta("AAMkInbox="), 200, INBOX_RESYNC);
    let arrivals = h.sync().unwrap();
    assert!(
        arrivals.is_empty(),
        "a resync announces nothing already cached"
    );
    assert!(h.message("AAMkMsgA=").is_some());
    assert!(
        h.message("AAMkMsgB=").is_none(),
        "B is gone from the server"
    );
    assert_eq!(
        h.store
            .remote_mailbox("AAMkInbox=")
            .unwrap()
            .unwrap()
            .sync_cursor
            .as_deref(),
        Some(INBOX_CURSOR_3)
    );
}

#[test]
fn server_throttling_fails_the_sync_with_a_category_only() {
    let mut h = Harness::new();
    h.fake.replace(
        Method::Post,
        &url("/$batch"),
        429,
        br#"{"error":{"code":"ApplicationThrottled","message":"Application is over its MailboxConcurrency limit."}}"#,
    );
    let error = h.sync().unwrap_err();
    assert!(matches!(error, Error::Remote(RemoteFailure::Throttled)));
    assert!(!error.to_string().contains("MailboxConcurrency"));
}

#[test]
fn fetch_one_and_prefetch_download_mime_without_marking_read() {
    let mut h = Harness::new();
    h.sync().unwrap();
    let a = h.message("AAMkMsgA=").unwrap();
    let source = b"From: Ada Lovelace <ada@example.test>\r\nSubject: Lunch on Friday\r\n\r\nShall we try the new place by the station?\r\n";
    h.fake.on(
        Method::Get,
        &url("/me/messages/AAMkMsgA%3D/$value"),
        200,
        source,
    );
    h.fake.on(
        Method::Get,
        &url("/me/messages/AAMkMsgB%3D/$value"),
        404,
        br#"{"error":{"code":"ErrorItemNotFound","message":"The specified object was not found in the store."}}"#,
    );
    assert_eq!(
        h.backend.fetch_one("Inbox", a.uid).unwrap().unwrap(),
        source.to_vec()
    );
    assert!(h.backend.fetch_one("Inbox", 1).unwrap().is_none());
    assert!(h
        .backend
        .fetch_one("No Such Folder", a.uid)
        .unwrap()
        .is_none());
    h.backend.prefetch_recent_bodies(&mut h.store).unwrap();
    let a = h.message("AAMkMsgA=").unwrap();
    assert!(a.body_hash.is_some());
    assert_eq!(a.flags, 0);
    assert!(h.message("AAMkMsgB=").unwrap().body_hash.is_none());
    assert!(
        h.fake.writes().is_empty(),
        "reading never writes to the server"
    );
}

#[test]
fn links_outside_graph_are_refused_and_the_token_never_prints() {
    let mut h = Harness::new();
    h.fake.replace(
        Method::Get,
        &initial_delta("AAMkInbox="),
        200,
        br#"{"value":[],"@odata.nextLink":"https://graph.microsoft.com.evil.example/v1.0/steal"}"#,
    );
    let error = h.sync().unwrap_err();
    assert!(matches!(error, Error::Remote(RemoteFailure::Protocol)));
    for bad in [
        "http://graph.microsoft.com/v1.0/me",
        "https://graph.microsoft.com.evil.example/v1.0/me",
        "https://evil.example/https://graph.microsoft.com/",
        "https://graph.microsoft.com/v1.0/me\r\nX-Injected: 1",
    ] {
        assert!(graph_link(bad).is_none(), "{bad}");
    }
    let refused = UreqTransport::new().send(
        &Request {
            method: Method::Get,
            url: "https://example.com/".into(),
            content_type: None,
            body: Vec::new(),
            prefer: None,
            max_body: 1,
        },
        &Secret::new(TOKEN.to_owned()),
    );
    assert_eq!(refused.unwrap_err(), HttpError::NotSent);
    let client = Client::new(h.fake.clone(), Secret::new(TOKEN.to_owned()));
    let diagnostics = format!(
        "{client:?} {:?} {error} {error:?}",
        Request {
            method: Method::Post,
            url: url("/me/sendMail"),
            content_type: Some("text/plain"),
            body: TOKEN.as_bytes().to_vec(),
            prefer: None,
            max_body: 1,
        }
    );
    assert!(!diagnostics.contains(TOKEN));
}

#[test]
fn outbox_requeues_definite_refusals_and_holds_ambiguous_ones() {
    let root = std::env::temp_dir().join(format!("rmac-mail-graph-outbox-{}", Uuid::new_v4()));
    let mut store = MailStorage::open(&root, Uuid::new_v4()).unwrap();
    let fake = Arc::new(Fake::default());
    let client = Client::new(fake.clone(), Secret::new(TOKEN.to_owned()));
    let mime = b"From: test@example.test\r\nTo: to@example.test\r\nSubject: Hi\r\n\r\nHello\r\n";
    store
        .queue_outbox("test@example.test", &["to@example.test".into()], mime)
        .unwrap();
    fake.on(Method::Post, &url("/me/sendMail"), 429, b"{}");
    assert!(matches!(
        drain_outbox(&client, &mut store),
        Err(Error::Remote(RemoteFailure::Throttled))
    ));
    assert_eq!(
        store.outbox_entries().unwrap()[0].state,
        OutboxState::Queued
    );
    fake.replace(
        Method::Post,
        &url("/me/sendMail"),
        400,
        br#"{"error":{"code":"ErrorInvalidRecipients","message":"At least one recipient isn't valid."}}"#,
    );
    assert!(matches!(
        drain_outbox(&client, &mut store),
        Err(Error::Remote(RemoteFailure::Rejected))
    ));
    assert_eq!(store.outbox_entries().unwrap()[0].state, OutboxState::Held);
    // Held messages wait for the person; nothing else is sent.
    assert_eq!(drain_outbox(&client, &mut store).unwrap(), 0);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bcc_and_model_helpers() {
    let mime = b"From: a@example.test\r\nTo: B <b@example.test>\r\nCc: c@example.test\r\n\r\nHi";
    assert_eq!(
        with_bcc(mime, &["B@example.test".into(), "c@example.test".into()]),
        mime.to_vec(),
        "no hidden recipients, no header"
    );
    let with_hidden = with_bcc(mime, &["b@example.test".into(), "d@example.test".into()]);
    assert!(with_hidden.starts_with(b"Bcc: d@example.test\r\nFrom:"));
    let bad = with_bcc(mime, &["x@example.test\r\nX-Evil: 1".into()]);
    assert_eq!(bad, mime.to_vec());

    let message: model::Message = serde_json::from_value(json!({
        "id": "x",
        "isRead": true,
    }))
    .unwrap();
    assert_eq!(
        model::flags(&message, FLAG_FLAGGED | FLAG_DRAFT),
        FLAG_SEEN | FLAG_FLAGGED | FLAG_DRAFT,
        "omitted properties keep their cached bits"
    );
    assert_eq!(
        model::mailbox_text(&model::Recipient {
            email_address: Some(model::EmailAddress {
                name: Some("Bell\u{7}e".into()),
                address: Some("belle@example.test".into()),
            }),
        }),
        "Bell e <belle@example.test>"
    );
    assert!(model::unix_time(Some("not a date")).is_none());
    assert!(matches!(
        status_error(401),
        Error::Remote(RemoteFailure::Unauthorized)
    ));
    assert!(matches!(
        status_error(503),
        Error::Remote(RemoteFailure::Throttled)
    ));
    assert!(matches!(
        status_error(502),
        Error::Remote(RemoteFailure::Network)
    ));
    assert!(first_sync_done(Some(INBOX_CURSOR_1)));
    assert!(!first_sync_done(Some(NEXT_PAGE2)));
    assert!(!first_sync_done(None));
}
