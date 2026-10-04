//! Microsoft Graph mail backend (MAIL-9) for Microsoft 365, Outlook.com and
//! Hotmail accounts that GNOME Online Accounts' `ms_graph` provider holds.
//!
//! * Credentials: GOA owns the OAuth token. The factory asks GOA for a fresh
//!   access token when a worker connects; it lives only in memory, is sent
//!   only to `https://graph.microsoft.com/` over verified TLS, and is never
//!   logged or written to disk.
//! * Sync: one Graph delta query per folder (`messages/delta`), resumed from
//!   the stored delta link, so a refresh with no changes is one small request
//!   per folder. There is no push channel; `rmac-mail-runtime` decides when
//!   to sync (start, network back, window focus, organise action, and a
//!   five-minute deadline only while Mail runs — `GRAPH_REFRESH`).
//! * Bodies: delta carries headers and the preview; the full RFC 5322 source
//!   (`/$value`) downloads on open (`fetch_one`), right away for new Inbox
//!   mail, and for the newest messages per folder (`prefetch_recent_bodies`).
//! * Changes: read, flag, move and delete replay from the storage journal
//!   with `PATCH`/`move`/`DELETE`; the Outbox sends with `sendMail` (MIME).
//!
//! All calls block; they run on the account's sync worker, never on the UI
//! thread.

pub mod http;
pub mod model;
mod outbox;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rmac_accounts::Secret;
use rmac_mail_runtime::{
    Account, Backend, BackendFactory, Error, NewMail, RemoteFailure, Transport,
};
use rmac_mail_storage::{Change, MailStorage, NewMessage, FLAG_FLAGGED, FLAG_SEEN};
use serde::de::DeserializeOwned;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use http::{HttpError, HttpTransport, Method, Request, Response, UreqTransport, GRAPH_ROOT};
pub use outbox::{drain_outbox, with_bcc};

use http::{graph_link, segment, MAX_JSON_BYTES, MAX_MIME_BYTES};
use model::{address_list, clean, mailbox_text, BatchReply, Folder, Page};

/// Graph has no UIDVALIDITY; ids are stable (immutable ids), so every Graph
/// mailbox uses one constant value.
const GRAPH_UIDVALIDITY: i64 = 1;
/// Same bound as the IMAP backend: the newest 50 header-only messages per
/// folder download after a sync.
const PREFETCH_LIMIT: usize = 50;
/// Folders per account and nesting depth this backend follows.
const MAX_FOLDERS: usize = 1000;
const MAX_FOLDER_DEPTH: usize = 8;
/// Delta pages per folder per sync. A very large first sync resumes from the
/// stored next link on the following sync instead of running unbounded.
const MAX_PAGES: usize = 200;
const PAGE_PREFER: &str = "odata.maxpagesize=100";
const MESSAGE_SELECT: &str = "subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,isDraft,flag,bodyPreview,internetMessageId";
const FOLDER_QUERY: &str = "$select=id,displayName,childFolderCount&$top=100";

/// Graph's well-known folder names and the special-use role Mail shows them
/// with (Favourites, archive/junk/trash actions).
const WELL_KNOWN: [(&str, &str); 6] = [
    ("inbox", "\\Inbox"),
    ("drafts", "\\Drafts"),
    ("sentitems", "\\Sent"),
    ("deleteditems", "\\Trash"),
    ("junkemail", "\\Junk"),
    ("archive", "\\Archive"),
];

/// Looks up an access token for one account at connect time.
pub type TokenLookup = dyn Fn(&Account) -> Result<Secret, Error> + Send + Sync;

pub struct GraphFactory {
    http: Arc<dyn HttpTransport>,
    tokens: Arc<TokenLookup>,
    data_root: PathBuf,
}

impl GraphFactory {
    /// `data_root` is Mail's cache root, used by `fetch_one` to map a cached
    /// message back to its Graph id.
    pub fn new(http: Arc<dyn HttpTransport>, tokens: Arc<TokenLookup>, data_root: PathBuf) -> Self {
        Self {
            http,
            tokens,
            data_root,
        }
    }

    /// Production wiring: verified HTTPS and GOA's `GetAccessToken`
    /// (preceded by `EnsureCredentials`, which refreshes an expired token).
    #[cfg(target_os = "linux")]
    pub fn goa(data_root: PathBuf) -> Self {
        Self::new(
            Arc::new(UreqTransport::new()),
            Arc::new(|account| goa_token(&account.path)),
            data_root,
        )
    }
}

/// The current GOA access token for an `ms_graph` account object path.
#[cfg(target_os = "linux")]
pub fn goa_token(path: &str) -> Result<Secret, Error> {
    use rmac_accounts_linux::GoaApi;
    rmac_accounts_linux::goa::GoaBus::session()
        .and_then(|bus| bus.access_token(path))
        .map_err(|_| Error::Account)
}

impl BackendFactory for GraphFactory {
    fn connect(&self, account: &Account) -> Result<Box<dyn Backend>, Error> {
        if !matches!(account.transport, Transport::Graph) {
            return Err(Error::Unsupported(
                "this account does not use Microsoft Graph",
            ));
        }
        let token = (self.tokens)(account)?;
        Ok(Box::new(GraphBackend {
            client: Client::new(Arc::clone(&self.http), token),
            data_root: self.data_root.clone(),
            account: account.id,
        }))
    }
}

/// An authenticated Graph session for one account.
pub struct Client {
    http: Arc<dyn HttpTransport>,
    token: Secret,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Client([redacted])")
    }
}

fn remote(failure: RemoteFailure) -> Error {
    Error::Remote(failure)
}

/// Maps an unsuccessful HTTP status to a category. Server text is dropped.
pub fn status_error(status: u16) -> Error {
    remote(match status {
        401 | 403 => RemoteFailure::Unauthorized,
        429 | 503 => RemoteFailure::Throttled,
        500..=599 => RemoteFailure::Network,
        400..=499 => RemoteFailure::Rejected,
        _ => RemoteFailure::Protocol,
    })
}

fn http_error(error: HttpError) -> Error {
    remote(match error {
        HttpError::NotSent | HttpError::Interrupted => RemoteFailure::Network,
        HttpError::Body => RemoteFailure::Protocol,
    })
}

fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|_| remote(RemoteFailure::Protocol))
}

fn link(url: &str) -> Result<String, Error> {
    graph_link(url).ok_or(remote(RemoteFailure::Protocol))
}

impl Client {
    pub fn new(http: Arc<dyn HttpTransport>, token: Secret) -> Self {
        Self { http, token }
    }

    fn send(
        &self,
        method: Method,
        url: String,
        body: Option<(&'static str, Vec<u8>)>,
        prefer: Option<&'static str>,
        max_body: u64,
    ) -> Result<Response, HttpError> {
        let (content_type, body) = match body {
            Some((content_type, body)) => (Some(content_type), body),
            None => (None, Vec::new()),
        };
        self.http.send(
            &Request {
                method,
                url,
                content_type,
                body,
                prefer,
                max_body,
            },
            &self.token,
        )
    }

    /// `Ok(None)` for 404: the object is gone on the server.
    fn get_json<T: DeserializeOwned>(
        &self,
        url: String,
        prefer: Option<&'static str>,
    ) -> Result<Option<T>, Error> {
        let response = self
            .send(Method::Get, url, None, prefer, MAX_JSON_BYTES)
            .map_err(http_error)?;
        match response.status {
            404 => Ok(None),
            status if success(status) => parse(&response.body).map(Some),
            status => Err(status_error(status)),
        }
    }

    /// Sends a JSON body. `Ok(false)` for 404 (already gone).
    fn call_json(
        &self,
        method: Method,
        url: String,
        value: serde_json::Value,
    ) -> Result<bool, Error> {
        let body = (method != Method::Delete)
            .then(|| ("application/json", value.to_string().into_bytes()));
        let response = self
            .send(method, url, body, None, MAX_JSON_BYTES)
            .map_err(http_error)?;
        match response.status {
            404 => Ok(false),
            status if success(status) => Ok(true),
            status => Err(status_error(status)),
        }
    }

    /// The message's complete RFC 5322 source. Reading it never marks the
    /// message read on the server.
    pub fn mime(&self, message_id: &str) -> Result<Option<Vec<u8>>, Error> {
        let url = format!("{GRAPH_ROOT}/me/messages/{}/$value", segment(message_id));
        let response = self
            .send(Method::Get, url, None, None, MAX_MIME_BYTES)
            .map_err(http_error)?;
        match response.status {
            404 => Ok(None),
            status if success(status) => Ok(Some(response.body)),
            status => Err(status_error(status)),
        }
    }
}

#[derive(Clone, Debug)]
struct FolderInfo {
    mailbox_id: i64,
    remote_id: String,
    special: Option<&'static str>,
    cursor: Option<String>,
}

struct GraphBackend {
    client: Client,
    data_root: PathBuf,
    account: Uuid,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// A stable positive UID for a Graph id, so a full resync maps the same
/// message to the same row. Probes past the rare collision.
fn uid_for(store: &MailStorage, mailbox_id: i64, remote_id: &str) -> Result<i64, Error> {
    let digest = Sha256::digest(remote_id.as_bytes());
    let mut raw = [0_u8; 8];
    raw[2..].copy_from_slice(&digest[..6]);
    let mut uid = (u64::from_be_bytes(raw) & ((1 << 47) - 1)) as i64 + 1;
    for _ in 0..64 {
        if store.message_by_uid(mailbox_id, uid)?.is_none() {
            return Ok(uid);
        }
        uid += 1;
    }
    Err(remote(RemoteFailure::Protocol))
}

fn initial_delta(remote_id: &str) -> String {
    format!(
        "{GRAPH_ROOT}/me/mailFolders/{}/messages/delta?$select={MESSAGE_SELECT}",
        segment(remote_id)
    )
}

/// A stored cursor is either a next link (first sync still paging) or a
/// delta link (first sync finished). Only after the latter is a message new.
fn first_sync_done(cursor: Option<&str>) -> bool {
    cursor.is_some_and(|cursor| cursor.contains("deltatoken="))
}

impl GraphBackend {
    fn well_known(&self) -> Result<HashMap<String, &'static str>, Error> {
        let requests: Vec<_> = WELL_KNOWN
            .iter()
            .map(|(name, _)| {
                json!({
                    "id": name,
                    "method": "GET",
                    "url": format!("/me/mailFolders/{name}?$select=id"),
                })
            })
            .collect();
        let body = json!({ "requests": requests }).to_string().into_bytes();
        let response = self
            .client
            .send(
                Method::Post,
                format!("{GRAPH_ROOT}/$batch"),
                Some(("application/json", body)),
                None,
                MAX_JSON_BYTES,
            )
            .map_err(http_error)?;
        if !success(response.status) {
            return Err(status_error(response.status));
        }
        let reply: BatchReply = parse(&response.body)?;
        let mut map = HashMap::new();
        for item in reply.responses {
            let Some(role) = WELL_KNOWN
                .iter()
                .find(|(name, _)| *name == item.id)
                .map(|(_, role)| *role)
            else {
                continue;
            };
            // A missing well-known folder (404, e.g. no Archive yet) is normal.
            if !success(item.status) {
                continue;
            }
            if let Some(id) = item
                .body
                .as_ref()
                .and_then(|body| body.get("id"))
                .and_then(|id| id.as_str())
            {
                map.insert(id.to_owned(), role);
            }
        }
        Ok(map)
    }

    /// `(graph id, "Parent/Child" path)` for every visible folder.
    fn list_folders(&self) -> Result<Vec<(String, String)>, Error> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([(
            format!("{GRAPH_ROOT}/me/mailFolders?{FOLDER_QUERY}"),
            None::<String>,
            0_usize,
        )]);
        while let Some((first, prefix, depth)) = queue.pop_front() {
            let mut next = Some(first);
            while let Some(url) = next.take() {
                let page: Page<Folder> = self
                    .client
                    .get_json(url, None)?
                    .ok_or(remote(RemoteFailure::Protocol))?;
                for folder in page.value {
                    if out.len() >= MAX_FOLDERS {
                        return Ok(out);
                    }
                    let name = clean(folder.display_name.as_deref().unwrap_or(""));
                    let name = if name.is_empty() {
                        "Untitled".to_owned()
                    } else {
                        name
                    };
                    let path = match &prefix {
                        Some(prefix) => format!("{prefix}/{name}"),
                        None => name,
                    };
                    if folder.child_folder_count.unwrap_or(0) > 0 && depth < MAX_FOLDER_DEPTH {
                        queue.push_back((
                            format!(
                                "{GRAPH_ROOT}/me/mailFolders/{}/childFolders?{FOLDER_QUERY}",
                                segment(&folder.id)
                            ),
                            Some(path.clone()),
                            depth + 1,
                        ));
                    }
                    out.push((folder.id, path));
                }
                next = page.next_link.as_deref().map(link).transpose()?;
            }
        }
        Ok(out)
    }

    /// Mirrors the server's folder tree into the cache: binds new folders,
    /// follows renames, and drops folders deleted on the server.
    fn sync_folders(&self, store: &mut MailStorage) -> Result<Vec<FolderInfo>, Error> {
        let specials = self.well_known()?;
        let listed = self.list_folders()?;
        let mut seen = HashSet::new();
        let mut folders = Vec::new();
        for (remote_id, path) in listed {
            let special = specials.get(&remote_id).copied();
            let mailbox_id = match store.remote_mailbox(&remote_id)? {
                Some(bound) => {
                    let name = if bound.name != path
                        && store.rename_mailbox(bound.mailbox_id, &path).is_ok()
                    {
                        path
                    } else {
                        bound.name
                    };
                    store.upsert_mailbox(&name, GRAPH_UIDVALIDITY, special)?
                }
                None => {
                    let id = store.upsert_mailbox(&path, GRAPH_UIDVALIDITY, special)?;
                    store.bind_remote_mailbox(id, &remote_id)?;
                    id
                }
            };
            let cursor = store
                .remote_mailbox(&remote_id)?
                .and_then(|bound| bound.sync_cursor);
            seen.insert(remote_id.clone());
            folders.push(FolderInfo {
                mailbox_id,
                remote_id,
                special,
                cursor,
            });
        }
        for bound in store.remote_mailboxes()? {
            if !seen.contains(&bound.remote_id) {
                store.remove_mailbox(bound.mailbox_id)?;
            }
        }
        Ok(folders)
    }

    /// Sends queued read/flag/move/delete changes. A change to a message the
    /// server no longer has is dropped; a local-only draft has nothing to
    /// send.
    fn replay(&self, store: &mut MailStorage, folders: &[FolderInfo]) -> Result<(), Error> {
        let trash: HashSet<i64> = folders
            .iter()
            .filter(|folder| folder.special == Some("\\Trash"))
            .map(|folder| folder.mailbox_id)
            .collect();
        for pending in store.pending_changes()? {
            let removes_source = matches!(pending.change, Change::Move(_) | Change::Delete);
            let Some(remote_id) = store.remote_id_of_message(pending.message_id)? else {
                store.discard_change(pending.id)?;
                if matches!(pending.change, Change::Delete) {
                    store.remove_server_uid(pending.mailbox_id, pending.uid)?;
                }
                continue;
            };
            let message_url = format!("{GRAPH_ROOT}/me/messages/{}", segment(&remote_id));
            match pending.change {
                Change::SetFlags(bits) => {
                    let flag_status = if bits & FLAG_FLAGGED != 0 {
                        "flagged"
                    } else {
                        "notFlagged"
                    };
                    self.client.call_json(
                        Method::Patch,
                        message_url,
                        json!({
                            "isRead": bits & FLAG_SEEN != 0,
                            "flag": { "flagStatus": flag_status },
                        }),
                    )?;
                }
                Change::Move(target) => {
                    let Some(destination) = store.remote_id_of_mailbox(target)? else {
                        // The target folder no longer exists on the server.
                        store.discard_change(pending.id)?;
                        continue;
                    };
                    self.client.call_json(
                        Method::Post,
                        format!("{message_url}/move"),
                        json!({ "destinationId": destination }),
                    )?;
                }
                Change::Delete if trash.contains(&pending.mailbox_id) => {
                    self.client
                        .call_json(Method::Delete, message_url, serde_json::Value::Null)?;
                }
                Change::Delete => {
                    // Like the Mac: Delete moves to Deleted Items; deleting
                    // from Deleted Items removes it for good.
                    self.client.call_json(
                        Method::Post,
                        format!("{message_url}/move"),
                        json!({ "destinationId": "deleteditems" }),
                    )?;
                }
            }
            store.discard_change(pending.id)?;
            if removes_source {
                store.remove_server_uid(pending.mailbox_id, pending.uid)?;
            }
        }
        Ok(())
    }

    fn apply(
        &self,
        store: &mut MailStorage,
        account: Uuid,
        folder: &FolderInfo,
        message: &model::Message,
        arrivals: bool,
    ) -> Result<Option<NewMail>, Error> {
        if let Some(existing) = store.message_by_remote_id(&message.id)? {
            if existing.mailbox_id == folder.mailbox_id {
                store.set_server_flags(
                    folder.mailbox_id,
                    existing.uid,
                    model::flags(message, existing.flags),
                )?;
                return Ok(None);
            }
            // The same immutable id in another folder: it moved here.
            store.remove_server_uid(existing.mailbox_id, existing.uid)?;
        }
        let uid = uid_for(store, folder.mailbox_id, &message.id)?;
        let bits = model::flags(message, 0);
        let body = if arrivals {
            self.client.mime(&message.id)?
        } else {
            None
        };
        let parsed = body
            .as_deref()
            .and_then(|bytes| rmac_mail_mime::parse(bytes).ok());
        let subject = clean(message.subject.as_deref().unwrap_or(""));
        let sender = message.from.as_ref().map(mailbox_text).unwrap_or_default();
        let recipients = address_list(message.to_recipients.as_ref());
        let cc = address_list(message.cc_recipients.as_ref());
        let preview = clean(message.body_preview.as_deref().unwrap_or(""));
        let message_id = message
            .internet_message_id
            .as_deref()
            .map(|id| {
                clean(id)
                    .trim_matches(|ch| ch == '<' || ch == '>')
                    .to_owned()
            })
            .filter(|id| !id.is_empty());
        let references: Vec<&str> = parsed
            .as_ref()
            .map(|parsed| parsed.references.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let id = store.put_message(&NewMessage {
            mailbox_id: folder.mailbox_id,
            uid,
            message_id: message_id.as_deref(),
            in_reply_to: parsed
                .as_ref()
                .and_then(|parsed| parsed.in_reply_to.as_deref()),
            references: &references,
            subject: &subject,
            sender: &sender,
            recipients: &recipients,
            cc: &cc,
            preview: &preview,
            received_at: model::unix_time(message.received_date_time.as_deref())
                .unwrap_or_else(now),
            flags: bits,
            body: parsed.as_ref().and(body.as_deref()),
            body_text: parsed.as_ref().map(|parsed| parsed.plain_text.as_str()),
        })?;
        store.bind_remote_message(id, &message.id)?;
        if arrivals && bits & FLAG_SEEN == 0 {
            let display = message
                .from
                .as_ref()
                .and_then(|from| from.email_address.as_ref())
                .and_then(|email| email.name.as_deref().or(email.address.as_deref()))
                .map(clean)
                .unwrap_or_default();
            return Ok(Some(NewMail {
                account,
                message_id: id,
                sender: display,
                subject,
                preview,
            }));
        }
        Ok(None)
    }

    fn sync_folder(
        &self,
        store: &mut MailStorage,
        account: Uuid,
        folder: &FolderInfo,
    ) -> Result<Vec<NewMail>, Error> {
        let arrivals =
            folder.special == Some("\\Inbox") && first_sync_done(folder.cursor.as_deref());
        let initial = initial_delta(&folder.remote_id);
        let mut url = folder.cursor.clone().unwrap_or_else(|| initial.clone());
        let mut reset = false;
        let mut seen = HashSet::new();
        let mut new_mail = Vec::new();
        for _ in 0..MAX_PAGES {
            let response = self
                .client
                .send(
                    Method::Get,
                    url.clone(),
                    None,
                    Some(PAGE_PREFER),
                    MAX_JSON_BYTES,
                )
                .map_err(http_error)?;
            // 410 Gone (or 400 for a malformed/expired token) means Graph
            // dropped this cursor: start over once and reconcile.
            if !reset && url != initial && matches!(response.status, 400 | 410) {
                reset = true;
                store.set_remote_cursor(folder.mailbox_id, None)?;
                url = initial.clone();
                continue;
            }
            if response.status == 404 {
                // The folder vanished mid-sync; the next folder listing drops it.
                return Ok(new_mail);
            }
            if !success(response.status) {
                return Err(status_error(response.status));
            }
            let page: Page<model::Message> = parse(&response.body)?;
            for message in &page.value {
                if message.removed.is_some() {
                    if let Some(existing) = store.message_by_remote_id(&message.id)? {
                        if existing.mailbox_id == folder.mailbox_id {
                            store.remove_server_uid(existing.mailbox_id, existing.uid)?;
                        }
                    }
                    continue;
                }
                if reset {
                    seen.insert(message.id.clone());
                }
                if let Some(arrival) = self.apply(store, account, folder, message, arrivals)? {
                    new_mail.push(arrival);
                }
            }
            if let Some(next) = page.next_link.as_deref() {
                url = link(next)?;
                store.set_remote_cursor(folder.mailbox_id, Some(&url))?;
                continue;
            }
            let delta = link(
                page.delta_link
                    .as_deref()
                    .ok_or(remote(RemoteFailure::Protocol))?,
            )?;
            if reset {
                for (uid, remote_id) in store.remote_uids(folder.mailbox_id)? {
                    if !seen.contains(&remote_id) {
                        store.remove_server_uid(folder.mailbox_id, uid)?;
                    }
                }
            }
            store.set_remote_cursor(folder.mailbox_id, Some(&delta))?;
            return Ok(new_mail);
        }
        // Page budget spent; the stored next link resumes on the next sync.
        Ok(new_mail)
    }
}

impl Backend for GraphBackend {
    fn sync(&mut self, store: &mut MailStorage, account: Uuid) -> Result<Vec<NewMail>, Error> {
        let folders = self.sync_folders(store)?;
        self.replay(store, &folders)?;
        // Best effort: a held or throttled send must not stop receiving.
        let _ = drain_outbox(&self.client, store);
        let mut new_mail = Vec::new();
        for folder in &folders {
            new_mail.extend(self.sync_folder(store, account, folder)?);
        }
        Ok(new_mail)
    }

    /// Never called for Graph accounts: the runtime owns the refresh
    /// schedule (`GRAPH_REFRESH`) and blocks on its command channel.
    fn wait_for_push(&mut self, _duration: Duration) -> Result<(), Error> {
        Ok(())
    }

    fn fetch_one(&mut self, mailbox_name: &str, uid: i64) -> Result<Option<Vec<u8>>, Error> {
        let remote_id = {
            let store = MailStorage::open(&self.data_root, self.account)?;
            let Some(mailbox) = store.mailbox(mailbox_name)? else {
                return Ok(None);
            };
            let Some(summary) = store.message_by_uid(mailbox.id, uid)? else {
                return Ok(None);
            };
            store.remote_id_of_message(summary.id)?
        };
        match remote_id {
            Some(remote_id) => self.client.mime(&remote_id),
            None => Ok(None),
        }
    }

    fn prefetch_recent_bodies(&mut self, store: &mut MailStorage) -> Result<(), Error> {
        for bound in store.remote_mailboxes()? {
            for uid in store.uids_missing_body(bound.mailbox_id, PREFETCH_LIMIT)? {
                let Some(summary) = store.message_by_uid(bound.mailbox_id, uid)? else {
                    continue;
                };
                let Some(remote_id) = store.remote_id_of_message(summary.id)? else {
                    continue;
                };
                let Some(bytes) = self.client.mime(&remote_id)? else {
                    continue;
                };
                if let Ok(parsed) = rmac_mail_mime::parse(&bytes) {
                    store.attach_body(summary.id, &bytes, &parsed.plain_text)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
