use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "linux")]
use rmac_accounts_linux::GoaApi;
use rmac_mail_imap::{
    Authentication, Client, Config, Interrupt, MailboxKind, Secret, SyncCursor, TlsMode,
};
use rmac_mail_storage::{
    Change, MailStorage, NewMessage, FLAG_ANSWERED, FLAG_DRAFT, FLAG_FLAGGED, FLAG_SEEN,
};
use uuid::Uuid;

use crate::{Account, Backend, BackendFactory, Error, NewMail, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImapAuth {
    XOAuth2,
    Password,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImapSettings {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub user: String,
    pub auth: ImapAuth,
}

impl std::fmt::Debug for ImapSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

/// Credential lookup is done at connection time, allowing GOA to refresh an
/// OAuth token without persisting it in Mail's cache.
pub type CredentialLookup = dyn Fn(&Account, ImapAuth) -> Result<Secret, Error> + Send + Sync;

pub struct ImapFactory {
    credentials: Arc<CredentialLookup>,
}

impl ImapFactory {
    pub fn new(credentials: Arc<CredentialLookup>) -> Self {
        Self { credentials }
    }

    #[cfg(target_os = "linux")]
    pub fn goa() -> Self {
        Self::new(Arc::new(|account, auth| {
            let bus = rmac_accounts_linux::goa::GoaBus::session().map_err(|_| Error::Account)?;
            let value = match auth {
                ImapAuth::XOAuth2 => bus.access_token(&account.path),
                ImapAuth::Password => bus.password(&account.path, "imap-password"),
            }
            .map_err(|_| Error::Account)?;
            Ok(Secret::new(value.expose()))
        }))
    }
}

impl BackendFactory for ImapFactory {
    fn connect(&self, account: &Account) -> Result<Box<dyn Backend>, Error> {
        let Transport::Imap(settings) = &account.transport else {
            return Err(Error::GraphUnavailable);
        };
        let config = Config {
            host: settings.host.clone(),
            port: settings.port,
            tls: settings.tls,
            timeout: Duration::from_secs(30),
        };
        let mut client = Client::connect(&config)?;
        let credential = (self.credentials)(account, settings.auth)?;
        let auth = match settings.auth {
            ImapAuth::XOAuth2 => Authentication::XOAuth2 {
                user: &settings.user,
                token: &credential,
            },
            ImapAuth::Password => Authentication::Plain {
                user: &settings.user,
                password: &credential,
            },
        };
        client.authenticate(auth)?;
        Ok(Box::new(ImapBackend {
            client,
            trash: None,
        }))
    }
}

struct ImapBackend {
    client: Client,
    trash: Option<String>,
}

fn flags_from_imap(flags: &[String]) -> i64 {
    let mut bits = 0;
    for flag in flags {
        if flag.eq_ignore_ascii_case("\\Seen") {
            bits |= FLAG_SEEN;
        }
        if flag.eq_ignore_ascii_case("\\Answered") {
            bits |= FLAG_ANSWERED;
        }
        if flag.eq_ignore_ascii_case("\\Flagged") {
            bits |= FLAG_FLAGGED;
        }
        if flag.eq_ignore_ascii_case("\\Draft") {
            bits |= FLAG_DRAFT;
        }
    }
    bits
}

fn flags_to_imap(flags: i64) -> Vec<&'static str> {
    let mut result = Vec::new();
    for (bit, name) in [
        (FLAG_SEEN, "\\Seen"),
        (FLAG_ANSWERED, "\\Answered"),
        (FLAG_FLAGGED, "\\Flagged"),
        (FLAG_DRAFT, "\\Draft"),
    ] {
        if flags & bit != 0 {
            result.push(name);
        }
    }
    result
}

fn special_use(kind: MailboxKind) -> Option<&'static str> {
    match kind {
        MailboxKind::Inbox => Some("\\Inbox"),
        MailboxKind::Drafts => Some("\\Drafts"),
        MailboxKind::Sent => Some("\\Sent"),
        MailboxKind::Trash => Some("\\Trash"),
        MailboxKind::Junk => Some("\\Junk"),
        MailboxKind::Archive => Some("\\Archive"),
        MailboxKind::Other => None,
    }
}

fn uid_set_members(input: &str) -> Vec<i64> {
    let mut result = Vec::new();
    for part in input.split(',') {
        if result.len() >= 4096 {
            break;
        }
        if let Some((a, b)) = part.split_once(':') {
            if let (Ok(start), Ok(end)) = (a.parse::<i64>(), b.parse::<i64>()) {
                if start > 0 && end >= start && end - start <= 4096 {
                    result.extend((start..=end).take(4096 - result.len()));
                }
            }
        } else if let Ok(uid) = part.parse::<i64>() {
            if uid > 0 {
                result.push(uid);
            }
        }
    }
    result
}

impl ImapBackend {
    fn replay(
        &mut self,
        store: &mut MailStorage,
        mailbox_id: i64,
        uidvalidity: i64,
    ) -> Result<(), Error> {
        for pending in store
            .pending_changes()?
            .into_iter()
            .filter(|item| item.mailbox_id == mailbox_id)
        {
            if pending.uidvalidity != uidvalidity {
                return Err(Error::StaleUidValidity);
            }
            let uid = u32::try_from(pending.uid).map_err(|_| Error::StaleUidValidity)?;
            let removes_source = matches!(&pending.change, Change::Move(_) | Change::Delete);
            match pending.change {
                Change::SetFlags(bits) => self.client.store_flags(uid, &flags_to_imap(bits))?,
                Change::Move(target) => {
                    let destination = store
                        .mailbox_by_id(target)?
                        .ok_or(Error::StaleUidValidity)?;
                    self.client.move_uids(&uid.to_string(), &destination.name)?;
                }
                Change::Delete => {
                    if let Some(trash) = &self.trash {
                        self.client.move_uids(&uid.to_string(), trash)?;
                    } else {
                        if !self.client.capabilities().has("UIDPLUS") {
                            return Err(rmac_mail_imap::Error::Unsupported(
                                "IMAP server cannot safely expunge one message",
                            )
                            .into());
                        }
                        self.client.mark_deleted(uid)?;
                        self.client.expunge_uids(&uid.to_string())?;
                    }
                }
            }
            store.acknowledge_change(pending.id, uidvalidity)?;
            if removes_source {
                store.remove_server_uid(mailbox_id, pending.uid)?;
            }
        }
        Ok(())
    }

    fn sync_mailbox(
        &mut self,
        store: &mut MailStorage,
        account: Uuid,
        name: &str,
        kind: MailboxKind,
    ) -> Result<Vec<NewMail>, Error> {
        let previous = store.mailbox(name)?;
        let cursor = previous.as_ref().and_then(|state| {
            (state.uidvalidity > 0 && state.highest_modseq > 0).then_some(SyncCursor {
                uid_validity: state.uidvalidity as u64,
                highest_modseq: state.highest_modseq as u64,
            })
        });
        let selected = self.client.select(name, cursor.as_ref())?;
        let uidvalidity = i64::try_from(selected.uid_validity.ok_or(Error::StaleUidValidity)?)
            .map_err(|_| Error::StaleUidValidity)?;
        if previous
            .as_ref()
            .is_some_and(|old| old.uidvalidity != uidvalidity)
        {
            return Err(Error::StaleUidValidity);
        }
        let mailbox_id = store.upsert_mailbox(name, uidvalidity, special_use(kind))?;
        self.replay(store, mailbox_id, uidvalidity)?;
        for set in &selected.vanished {
            for uid in uid_set_members(set) {
                store.remove_server_uid(mailbox_id, uid)?;
            }
        }
        let qresync = self.client.capabilities().has("QRESYNC");
        let changes = self.client.fetch_changes(if qresync {
            previous
                .as_ref()
                .and_then(|old| (old.highest_modseq > 0).then_some(old.highest_modseq as u64))
        } else {
            None
        })?;
        if !qresync {
            let present: HashSet<i64> =
                changes.iter().map(|change| i64::from(change.uid)).collect();
            for uid in store.cached_uids(mailbox_id)? {
                if !present.contains(&uid) {
                    store.remove_server_uid(mailbox_id, uid)?;
                }
            }
        }
        let mut new_mail = Vec::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        for change in changes.into_iter().chain(selected.changed) {
            let uid = i64::from(change.uid);
            let bits = flags_from_imap(&change.flags);
            if store.message_by_uid(mailbox_id, uid)?.is_some() {
                store.set_server_flags(mailbox_id, uid, bits)?;
                continue;
            }
            let fetch_full = kind == MailboxKind::Inbox && previous.is_some();
            let Some(bytes) = (if fetch_full {
                self.client.fetch_body(change.uid)?
            } else {
                self.client.fetch_headers(change.uid)?
            }) else {
                continue;
            };
            let parsed = rmac_mail_mime::parse(&bytes)?;
            let refs: Vec<_> = parsed.references.iter().map(String::as_str).collect();
            let sender = parsed
                .from
                .first()
                .map(|from| from.display_name.as_deref().unwrap_or(&from.address))
                .unwrap_or("");
            let recipients = parsed
                .to
                .iter()
                .map(|to| to.address.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let id = store.put_message(&NewMessage {
                mailbox_id,
                uid,
                message_id: parsed.message_id.as_deref(),
                in_reply_to: parsed.in_reply_to.as_deref(),
                references: &refs,
                subject: &parsed.subject,
                sender,
                recipients: &recipients,
                preview: &parsed.preview,
                received_at: now,
                flags: bits,
                body: fetch_full.then_some(bytes.as_slice()),
                body_text: fetch_full.then_some(parsed.plain_text.as_str()),
            })?;
            if kind == MailboxKind::Inbox && previous.is_some() && bits & FLAG_SEEN == 0 {
                new_mail.push(NewMail {
                    account,
                    message_id: id,
                    sender: sender.into(),
                    subject: parsed.subject,
                    preview: parsed.preview,
                });
            }
        }
        if let Some(modseq) = selected
            .highest_modseq
            .and_then(|value| i64::try_from(value).ok())
        {
            store.set_mailbox_modseq(mailbox_id, modseq)?;
        }
        Ok(new_mail)
    }
}

impl Backend for ImapBackend {
    fn sync(&mut self, store: &mut MailStorage, account: Uuid) -> Result<Vec<NewMail>, Error> {
        let mailboxes = self.client.list_mailboxes()?;
        self.trash = mailboxes
            .iter()
            .find(|mailbox| mailbox.kind == MailboxKind::Trash && mailbox.selectable)
            .map(|mailbox| mailbox.name.clone());
        let mut all = Vec::new();
        for mailbox in mailboxes.into_iter().filter(|mailbox| mailbox.selectable) {
            all.extend(self.sync_mailbox(store, account, &mailbox.name, mailbox.kind)?);
        }
        // IDLE always watches Inbox, regardless of LIST order.
        self.client.select("INBOX", None)?;
        Ok(all)
    }

    fn wait_for_push(&mut self, duration: Duration) -> Result<(), Error> {
        self.client.idle_once(duration)?;
        Ok(())
    }

    fn interrupt(&self) -> Option<Interrupt> {
        self.client.interrupt_handle().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        ServerConfig, ServerConnection, StreamOwned,
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    const CERT: &[u8] = include_bytes!("../../rmac-mail-imap/tests/fixtures/server.cert.der");
    const KEY: &[u8] = include_bytes!("../../rmac-mail-imap/tests/fixtures/server.key.der");
    const CA: &[u8] = include_bytes!("../../rmac-mail-imap/tests/fixtures/ca.der");

    fn line(stream: &mut impl Read) -> Option<String> {
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0];
            if stream.read_exact(&mut byte).is_err() {
                return None;
            }
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n") {
                return String::from_utf8(bytes).ok();
            }
        }
    }

    fn send(stream: &mut impl Write, response: &str) {
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn literal(stream: &mut impl Write, tag: &str, uid: u32, part: &str, bytes: &[u8]) {
        send(
            stream,
            &format!("* 1 FETCH (UID {uid} BODY[{part}] {{{}}}\r\n", bytes.len()),
        );
        stream.write_all(bytes).unwrap();
        send(stream, &format!(")\r\n{tag} OK done\r\n"));
    }

    #[test]
    fn fixture_sync_replays_flags_and_reconciles_deletion_without_qresync() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let config = Arc::new(
                ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(
                        vec![CertificateDer::from(CERT)],
                        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY)),
                    )
                    .unwrap(),
            );
            let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), socket);
            send(&mut stream, "* OK ready\r\n");
            let mut round = 0;
            while let Some(command) = line(&mut stream) {
                let (tag, rest) = command.trim_end().split_once(' ').unwrap();
                match rest {
                    "CAPABILITY" => send(
                        &mut stream,
                        &format!("* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n{tag} OK done\r\n"),
                    ),
                    "LIST \"\" \"*\"" => {
                        round += 1;
                        send(&mut stream, &format!("* LIST (\\HasNoChildren) \"/\" INBOX\r\n* LIST (\\Trash) \"/\" Trash\r\n{tag} OK done\r\n"));
                    }
                    "SELECT \"INBOX\"" => {
                        let count = if round == 1 || round == 4 { 1 } else { 2 };
                        send(&mut stream, &format!("* {count} EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n{tag} OK done\r\n"));
                    }
                    "SELECT \"Trash\"" => send(
                        &mut stream,
                        &format!("* 0 EXISTS\r\n* OK [UIDVALIDITY 43] valid\r\n{tag} OK done\r\n"),
                    ),
                    "UID FETCH 1:* (UID FLAGS)" => {
                        if round == 4 {
                            send(
                                &mut stream,
                                &format!("* 1 FETCH (UID 8 FLAGS (\\Seen))\r\n{tag} OK done\r\n"),
                            );
                        } else if round >= 3 {
                            send(&mut stream, &format!("* 1 FETCH (UID 7 FLAGS ())\r\n* 2 FETCH (UID 8 FLAGS (\\Seen))\r\n{tag} OK done\r\n"));
                        } else if round == 2 {
                            send(&mut stream, &format!("* 1 FETCH (UID 7 FLAGS ())\r\n* 2 FETCH (UID 8 FLAGS ())\r\n{tag} OK done\r\n"));
                        } else {
                            send(
                                &mut stream,
                                &format!("* 1 FETCH (UID 7 FLAGS ())\r\n{tag} OK done\r\n"),
                            );
                        }
                    }
                    "UID FETCH 7 (UID BODY.PEEK[HEADER])" => literal(
                        &mut stream,
                        tag,
                        7,
                        "HEADER",
                        b"From: Ada <ada@example.test>\r\nSubject: Existing\r\n\r\n",
                    ),
                    "UID FETCH 8 (UID BODY.PEEK[])" => literal(
                        &mut stream,
                        tag,
                        8,
                        "",
                        b"From: Bob <bob@example.test>\r\nSubject: New\r\n\r\nHello from fixture",
                    ),
                    "UID STORE 8 -FLAGS.SILENT (\\Seen \\Answered \\Flagged \\Draft)"
                    | "UID STORE 8 +FLAGS.SILENT (\\Seen)" => {
                        send(&mut stream, &format!("{tag} OK done\r\n"))
                    }
                    _ => panic!("unexpected fixture command"),
                }
            }
            assert_eq!(round, 4);
        });
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(CA)).unwrap();
        let client = Client::connect_with_roots(
            &Config {
                host: "127.0.0.1".into(),
                port,
                tls: TlsMode::Implicit,
                timeout: Duration::from_secs(3),
            },
            roots,
        )
        .unwrap();
        let mut backend = ImapBackend {
            client,
            trash: None,
        };
        let root = std::env::temp_dir().join(format!("mail-imap-runtime-{}", Uuid::new_v4()));
        let account = Uuid::new_v4();
        let mut store = MailStorage::open(&root, account).unwrap();
        assert!(backend.sync(&mut store, account).unwrap().is_empty());
        let inbox = store.mailbox("INBOX").unwrap().unwrap();
        assert!(store
            .message_by_uid(inbox.id, 7)
            .unwrap()
            .unwrap()
            .body_hash
            .is_none());
        let arrivals = backend.sync(&mut store, account).unwrap();
        assert_eq!(arrivals.len(), 1);
        assert_eq!(arrivals[0].subject, "New");
        assert!(store
            .message_by_uid(inbox.id, 8)
            .unwrap()
            .unwrap()
            .body_hash
            .is_some());
        store
            .queue_change(arrivals[0].message_id, Change::SetFlags(FLAG_SEEN))
            .unwrap();
        assert!(backend.sync(&mut store, account).unwrap().is_empty());
        assert!(store.pending_changes().unwrap().is_empty());
        assert!(backend.sync(&mut store, account).unwrap().is_empty());
        assert_eq!(store.cached_uids(inbox.id).unwrap(), vec![8]);
        drop(backend);
        server.join().unwrap();
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn standard_flags_round_trip_and_uid_sets_are_bounded() {
        let bits = FLAG_SEEN | FLAG_FLAGGED;
        assert_eq!(flags_to_imap(bits), vec!["\\Seen", "\\Flagged"]);
        assert_eq!(
            flags_from_imap(&["\\seen".into(), "\\FLAGGED".into()]),
            bits
        );
        assert_eq!(uid_set_members("2:4,9"), vec![2, 3, 4, 9]);
        assert!(uid_set_members("1:999999999").is_empty());
    }
}
