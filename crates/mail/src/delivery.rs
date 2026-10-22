//! Account discovery and outbound delivery. All calls in this module block and
//! must run outside the GPUI thread.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_accounts::provider::{Provider, SocketSecurity};
#[cfg(target_os = "linux")]
use rmac_accounts_linux::{goa::GoaBus, GoaApi};
#[cfg(windows)]
use rmac_accounts_linux::{
    winstore::{Kind, WinAccounts},
    GoaApi,
};
use rmac_mail::schedule::{Schedule, ScheduledSend};
use rmac_mail_mime::{build, draft_source, guess_content_type, Draft, Priority};
#[cfg(any(target_os = "linux", windows))]
use rmac_mail_runtime::account_id;
#[cfg(any(target_os = "linux", windows))]
use rmac_mail_smtp::Secret;
use rmac_mail_smtp::{drain_outbox, Authentication, Config, Security};
use rmac_mail_storage::{MailStorage, NewMessage, FLAG_DRAFT};
use uuid::Uuid;

#[derive(Clone)]
pub struct ComposeAccount {
    /// GOA's object path, used only to fetch credentials at send time. Read
    /// only on Linux; other platforms have no GOA to ask.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub path: String,
    pub id: Uuid,
    pub address: String,
    pub provider: String,
}

/// Startup discovery runs before GPUI. No credential is retained in the UI.
/// GOA is a Linux session service; other platforms have no accounts yet.
#[cfg(target_os = "linux")]
pub fn accounts() -> Vec<ComposeAccount> {
    GoaBus::session()
        .and_then(|goa| goa.accounts())
        .unwrap_or_default()
        .into_iter()
        .filter(|account| account.services.mail)
        .map(|account| ComposeAccount {
            path: account.path.clone(),
            id: account_id(&account),
            address: account.identity,
            provider: account.provider,
        })
        .collect()
}

/// Windows: the mail accounts in Lulo's own store (Credential Manager for
/// the secrets).
#[cfg(windows)]
pub fn accounts() -> Vec<ComposeAccount> {
    WinAccounts::session()
        .and_then(|store| store.stored())
        .unwrap_or_default()
        .into_iter()
        .filter(|account| account.mail)
        .map(|account| ComposeAccount {
            path: account.path(),
            id: account_id(&account.to_goa()),
            address: account.identity,
            provider: account.provider,
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn accounts() -> Vec<ComposeAccount> {
    Vec::new()
}

pub(crate) fn data_root() -> Option<PathBuf> {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(data_home).join("lulo/mail"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/lulo/mail"))
}

pub enum DeliveryResult {
    Sent,
    Queued,
    Failed(&'static str),
}

/// A compose window's local autosaved copy: a mailbox id plus the UID of the
/// row within it, reused across autosaves so each edit updates the same
/// draft instead of leaving a trail of earlier copies behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftLocation {
    pub mailbox_id: i64,
    pub uid: i64,
}

/// A file chosen through the attachment picker, read once off the GPUI
/// thread and kept in memory until the message sends or the window closes.
#[derive(Clone)]
pub struct PendingAttachment {
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    pub bytes: std::sync::Arc<Vec<u8>>,
}

/// Read a chosen attachment's bytes. Must run off the GPUI thread: this is
/// ordinary blocking file I/O.
pub fn read_attachment(path: &std::path::Path) -> Result<PendingAttachment, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Mail could not read {}: {error}", path.display()))?;
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Attachment".to_owned());
    let content_type = guess_content_type(&filename).to_owned();
    Ok(PendingAttachment {
        filename,
        content_type,
        size: bytes.len(),
        bytes: std::sync::Arc::new(bytes),
    })
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Autosave the fields of a compose window to the account's local Drafts
/// mailbox, debounced by the caller. The first save allocates a UID; every
/// later save for the same window reuses it so the row is replaced in
/// place, matching the Mac's single "most recent draft" per window.
/// What a draft carries beyond its addresses and text: View ▸ Reply-To
/// Address Field and Message ▸ Set Priority.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftHeaders {
    pub reply_to: Vec<String>,
    pub priority: Priority,
}

#[allow(clippy::too_many_arguments)]
pub fn save_draft(
    account: &ComposeAccount,
    existing: Option<DraftLocation>,
    from: &str,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    attachments: &[PendingAttachment],
    headers: &DraftHeaders,
) -> Option<DraftLocation> {
    let root = data_root()?;
    let mut storage = MailStorage::open(&root, account.id).ok()?;
    let mailbox_id = match existing {
        Some(location) => location.mailbox_id,
        None => storage.upsert_mailbox("Drafts", 0, Some("\\Drafts")).ok()?,
    };
    let uid = match existing {
        Some(location) => location.uid,
        None => storage.allocate_local_uid(mailbox_id).ok()?,
    };
    let recipients = to.iter().chain(bcc).cloned().collect::<Vec<_>>().join(", ");
    let cc_joined = cc.join(", ");
    let preview: String = body.chars().filter(|c| *c != '\n').take(160).collect();
    let subject = if subject.trim().is_empty() {
        "(No Subject)"
    } else {
        subject
    };
    // The draft's own source, so its Reply-To and priority are kept with
    // it (and View ▸ Message ▸ Raw Source shows what will be sent).
    let source = draft_source(&Draft {
        from: from.to_owned(),
        to: to.to_vec(),
        cc: cc.to_vec(),
        subject: subject.to_owned(),
        text: body.to_owned(),
        reply_to: headers.reply_to.clone(),
        priority: headers.priority,
        ..Draft::default()
    })
    .ok();
    let message_id = storage
        .put_message(&NewMessage {
            mailbox_id,
            uid,
            message_id: None,
            in_reply_to: None,
            references: &[],
            subject,
            sender: from,
            recipients: &recipients,
            cc: &cc_joined,
            preview: &preview,
            received_at: now_unix(),
            flags: FLAG_DRAFT,
            body: source.as_deref(),
            body_text: Some(body),
        })
        .ok()?;
    storage.clear_attachments(message_id).ok()?;
    for attachment in attachments {
        let _ = storage.put_attachment(
            message_id,
            &attachment.filename,
            &attachment.content_type,
            &attachment.bytes,
        );
    }
    Some(DraftLocation { mailbox_id, uid })
}

/// Drop a window's local draft row, once it has either sent or the person
/// discarded it. A draft that never autosaved has no location to remove.
pub fn discard_draft(account: &ComposeAccount, location: DraftLocation) {
    let Some(root) = data_root() else {
        return;
    };
    let Ok(mut storage) = MailStorage::open(&root, account.id) else {
        return;
    };
    let _ = storage.remove_server_uid(location.mailbox_id, location.uid);
}

/// The credential SMTP submission needs, fetched from GOA at send time so
/// nothing is retained in the UI. GOA is Linux-only; other platforms have
/// no SMTP credential source yet, so a send always queues to the Outbox.
#[cfg(target_os = "linux")]
fn smtp_authentication(account: &ComposeAccount) -> Option<Authentication> {
    let goa = GoaBus::session().ok()?;
    if account.provider == "google" {
        let token = goa.access_token(&account.path).ok()?;
        Some(Authentication::Xoauth2 {
            user: account.address.clone(),
            token: Secret::new(token.expose().to_owned()),
        })
    } else {
        let password = goa.password(&account.path, "smtp-password").ok()?;
        Some(Authentication::Plain {
            user: account.address.clone(),
            password: Secret::new(password.expose().to_owned()),
        })
    }
}

/// Windows: the stored SMTP password, or the OAuth token for Google.
#[cfg(windows)]
fn smtp_authentication(account: &ComposeAccount) -> Option<Authentication> {
    let store = WinAccounts::session().ok()?;
    let stored = store.stored_by_path(&account.path).ok()?;
    if stored.kind == Kind::OAuth {
        let token = store.access_token(&account.path).ok()?;
        Some(Authentication::Xoauth2 {
            user: account.address.clone(),
            token: Secret::new(token.expose().to_owned()),
        })
    } else {
        let password = store.password(&account.path, "smtp-password").ok()?;
        let user = stored
            .smtp
            .map_or_else(|| account.address.clone(), |smtp| smtp.username);
        Some(Authentication::Plain {
            user,
            password: Secret::new(password.expose().to_owned()),
        })
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn smtp_authentication(_account: &ComposeAccount) -> Option<Authentication> {
    None
}

/// The SMTP server to submit through: what the account stored on Windows
/// (a discovered or typed-in server), otherwise the provider's preset.
#[cfg(windows)]
fn smtp_server(account: &ComposeAccount) -> Option<Config> {
    let stored = WinAccounts::session()
        .ok()?
        .stored_by_path(&account.path)
        .ok()?;
    let smtp = stored.smtp.or_else(|| {
        let preset = Provider::Google.info().servers?;
        (stored.provider == "google").then(|| rmac_accounts_linux::winstore::StoredServer {
            host: preset.smtp_host.into(),
            port: preset.smtp_port,
            security: "starttls".into(),
            username: account.address.clone(),
        })
    })?;
    Some(Config {
        host: smtp.host.clone(),
        port: smtp.port,
        helo_name: "localhost".into(),
        security: match smtp.security() {
            SocketSecurity::Tls => Security::ImplicitTls,
            SocketSecurity::StartTls => Security::StartTls,
        },
    })
}

#[cfg(not(windows))]
fn smtp_server(account: &ComposeAccount) -> Option<Config> {
    let domain = account
        .address
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or_default();
    let provider = if account.provider == "google" {
        Provider::Google
    } else {
        Provider::from_domain(domain)
    };
    let server = provider.info().servers?;
    Some(Config {
        host: server.smtp_host.into(),
        port: server.smtp_port,
        helo_name: "localhost".into(),
        security: match server.smtp_security {
            SocketSecurity::Tls => Security::ImplicitTls,
            SocketSecurity::StartTls => Security::StartTls,
        },
    })
}

/// Microsoft accounts send through Graph's `sendMail` (MAIL-9) with GOA's
/// token, fetched now and dropped after the call. Anything that fails stays
/// in the Outbox; the account's sync worker retries queued messages.
#[cfg(target_os = "linux")]
fn graph_delivery(account: &ComposeAccount, storage: &mut MailStorage) -> DeliveryResult {
    let Ok(token) = rmac_mail_graph::goa_token(&account.path) else {
        return DeliveryResult::Queued;
    };
    let client = rmac_mail_graph::Client::new(
        std::sync::Arc::new(rmac_mail_graph::UreqTransport::new()),
        token,
    );
    match rmac_mail_graph::drain_outbox(&client, storage) {
        Ok(_) => DeliveryResult::Sent,
        Err(_) => DeliveryResult::Queued,
    }
}

#[cfg(windows)]
fn graph_delivery(account: &ComposeAccount, storage: &mut MailStorage) -> DeliveryResult {
    let Ok(token) = rmac_mail_graph::account_token(&account.path) else {
        return DeliveryResult::Queued;
    };
    let client = rmac_mail_graph::Client::new(
        std::sync::Arc::new(rmac_mail_graph::UreqTransport::new()),
        token,
    );
    match rmac_mail_graph::drain_outbox(&client, storage) {
        Ok(_) => DeliveryResult::Sent,
        Err(_) => DeliveryResult::Queued,
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn graph_delivery(_account: &ComposeAccount, _storage: &mut MailStorage) -> DeliveryResult {
    DeliveryResult::Queued
}

pub fn deliver(
    account: &ComposeAccount,
    mut draft: Draft,
    saved: Option<DraftLocation>,
) -> DeliveryResult {
    draft.from.clone_from(&account.address);
    let built = match build(&draft) {
        Ok(message) => message,
        Err(_) => return DeliveryResult::Failed("Check the message fields and attachments"),
    };
    send_built(
        account,
        &built.envelope_from,
        &built.envelope_to,
        &built.bytes,
        saved,
    )
}

/// Queues a finished message in the account's Outbox, removes its draft and
/// tries to send it right away.
fn send_built(
    account: &ComposeAccount,
    envelope_from: &str,
    envelope_to: &[String],
    bytes: &[u8],
    saved: Option<DraftLocation>,
) -> DeliveryResult {
    let Some(root) = data_root() else {
        return DeliveryResult::Failed("Mail could not find its data folder");
    };
    let mut storage = match MailStorage::open(&root, account.id) {
        Ok(storage) => storage,
        Err(_) => return DeliveryResult::Failed("Mail could not open the Outbox"),
    };
    if storage
        .queue_outbox(envelope_from, envelope_to, bytes)
        .is_err()
    {
        return DeliveryResult::Failed("Mail could not save the message to Outbox");
    }
    // The message now lives in the Outbox; the Mac removes a sent draft from
    // Drafts the moment it leaves, whether or not SMTP accepts it instantly.
    if let Some(location) = saved {
        discard_draft(account, location);
    }
    if account.provider == "ms_graph" {
        return graph_delivery(account, &mut storage);
    }
    let Some(auth) = smtp_authentication(account) else {
        return DeliveryResult::Queued;
    };
    let Some(config) = smtp_server(account) else {
        return DeliveryResult::Queued;
    };
    match drain_outbox(&mut storage, &config, &auth) {
        Ok(_) => DeliveryResult::Sent,
        Err(_) => DeliveryResult::Queued,
    }
}

/// Message ▸ Send Later: builds the message now (so later edits to the
/// window cannot change it), keeps it beside Mail's schedule and adds it
/// there. Its draft stays in Drafts until it is sent.
pub fn schedule_send(
    account: &ComposeAccount,
    mut draft: Draft,
    saved: Option<DraftLocation>,
    at: i64,
) -> Result<ScheduledSend, &'static str> {
    draft.from.clone_from(&account.address);
    let built = build(&draft).map_err(|_| "Check the message fields and attachments")?;
    let root = data_root().ok_or("Mail could not find its data folder")?;
    let entry = ScheduledSend {
        id: Uuid::new_v4(),
        account: account.id,
        at,
        subject: draft.subject.clone(),
        envelope_from: built.envelope_from,
        envelope_to: built.envelope_to,
        draft: saved.map(|location| (location.mailbox_id, location.uid)),
    };
    let path = Schedule::message_path(&root, entry.id);
    std::fs::create_dir_all(path.parent().unwrap_or(&root))
        .and_then(|_| std::fs::write(&path, &built.bytes))
        .map_err(|_| "Mail could not save the message for later")?;
    let mut schedule = Schedule::load(&root);
    schedule.sends.push(entry.clone());
    schedule
        .save(&root)
        .map_err(|_| "Mail could not save the message for later")?;
    Ok(entry)
}

/// Sends one Send Later message whose time has come. Its file is removed
/// once the message is in the Outbox.
pub fn send_scheduled(accounts: &[ComposeAccount], entry: &ScheduledSend) -> DeliveryResult {
    let Some(root) = data_root() else {
        return DeliveryResult::Failed("Mail could not find its data folder");
    };
    let path = Schedule::message_path(&root, entry.id);
    let Ok(bytes) = std::fs::read(&path) else {
        return DeliveryResult::Failed("The scheduled message is missing");
    };
    let Some(account) = accounts.iter().find(|account| account.id == entry.account) else {
        return DeliveryResult::Failed("The account for this message was removed");
    };
    let saved = entry
        .draft
        .map(|(mailbox_id, uid)| DraftLocation { mailbox_id, uid });
    let result = send_built(
        account,
        &entry.envelope_from,
        &entry.envelope_to,
        &bytes,
        saved,
    );
    if !matches!(result, DeliveryResult::Failed(_)) {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Where Mail keeps its Send Later and Remind Me schedule.
pub fn schedule_root() -> Option<PathBuf> {
    data_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_mail_mime::Draft;

    /// This test is the only one in the `rmac-mail` binary target reading
    /// `XDG_DATA_HOME` (`data_root`'s path). Rust's default test harness
    /// runs `#[test]` functions on separate threads within the same
    /// process, so two tests mutating this process-wide variable at once
    /// would race; `ENV_LOCK` serialises every `TempDataHome` so only one
    /// is ever active.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    struct TempDataHome {
        _guard: std::sync::MutexGuard<'static, ()>,
        previous: Option<std::ffi::OsString>,
        root: PathBuf,
    }
    impl TempDataHome {
        fn new(tag: &str) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let root = std::env::temp_dir().join(format!(
                "rmac-mail-delivery-test-{tag}-{}-{}",
                std::process::id(),
                tag.len()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let previous = std::env::var_os("XDG_DATA_HOME");
            std::env::set_var("XDG_DATA_HOME", &root);
            Self {
                _guard: guard,
                previous,
                root,
            }
        }
    }
    impl Drop for TempDataHome {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn account() -> ComposeAccount {
        ComposeAccount {
            path: "/fixture/test".to_owned(),
            id: Uuid::new_v4(),
            address: "jacob@example.test".to_owned(),
            provider: "imap_smtp".to_owned(),
        }
    }

    #[test]
    fn a_draft_autosave_is_readable_back_from_storage() {
        let home = TempDataHome::new("draft");
        let account = account();
        let location = save_draft(
            &account,
            None,
            &account.address,
            &["anna@example.test".to_owned()],
            &["sam@example.test".to_owned()],
            &[],
            "Lunch on Friday?",
            "Fancy lunch on Friday?",
            &[],
            &DraftHeaders {
                reply_to: vec!["team@example.test".to_owned()],
                priority: Priority::High,
            },
        )
        .expect("a fresh draft should save");

        let storage = MailStorage::open(&home.root.join("lulo/mail"), account.id).unwrap();
        let saved = storage
            .message_by_uid(location.mailbox_id, location.uid)
            .unwrap()
            .unwrap();
        assert_eq!(saved.subject, "Lunch on Friday?");
        assert_eq!(saved.flags & FLAG_DRAFT, FLAG_DRAFT);
        assert!(saved.recipients.contains("anna@example.test"));
        assert_eq!(saved.cc, "sam@example.test");
        // Reply-To and the priority are kept in the draft's source.
        let source = storage
            .read_blob(
                saved
                    .body_hash
                    .as_deref()
                    .expect("a draft keeps its source"),
            )
            .unwrap();
        let parsed = rmac_mail_mime::parse(&source).unwrap();
        assert_eq!(parsed.reply_to[0].address, "team@example.test");
        assert_eq!(parsed.priority, Priority::High);

        // Re-saving the same window's draft replaces the row in place
        // rather than leaving a trail of earlier autosaves behind.
        let second = save_draft(
            &account,
            Some(location),
            &account.address,
            &["anna@example.test".to_owned()],
            &[],
            &[],
            "Lunch on Friday?",
            "Fancy lunch on Friday? Say noon.",
            &[],
            &DraftHeaders::default(),
        )
        .expect("re-saving the same draft should succeed");
        assert_eq!(second, location, "a later autosave reuses the same row");
        let updated = storage.get_message(saved.id).unwrap().unwrap();
        assert!(updated.preview.contains("Say noon"));

        discard_draft(&account, location);
        assert!(storage
            .message_by_uid(location.mailbox_id, location.uid)
            .unwrap()
            .is_none());
    }

    #[test]
    fn sending_with_no_known_smtp_server_queues_to_outbox_and_drops_the_draft() {
        let home = TempDataHome::new("send");
        let account = account();
        let draft_location = save_draft(
            &account,
            None,
            &account.address,
            &["anna@example.test".to_owned()],
            &[],
            &[],
            "Hello",
            "Hi Anna",
            &[],
            &DraftHeaders::default(),
        )
        .unwrap();
        let draft = Draft {
            to: vec!["anna@example.test".to_owned()],
            subject: "Hello".to_owned(),
            text: "Hi Anna".to_owned(),
            ..Draft::default()
        };
        // `account.provider` is not "google" and its domain matches no
        // known preset, so `deliver` has no SMTP server to try and must
        // queue rather than silently drop the message.
        let result = deliver(&account, draft, Some(draft_location));
        let outcome = match &result {
            DeliveryResult::Sent => "sent".to_owned(),
            DeliveryResult::Queued => "queued".to_owned(),
            DeliveryResult::Failed(reason) => format!("failed: {reason}"),
        };
        assert_eq!(outcome, "queued");

        let storage = MailStorage::open(&home.root.join("lulo/mail"), account.id).unwrap();
        assert_eq!(
            storage
                .outbox_count(rmac_mail_storage::OutboxState::Queued)
                .unwrap(),
            1
        );
        // The Mac drops a sent draft from Drafts the moment it leaves,
        // whether or not SMTP has accepted it yet.
        assert!(storage
            .message_by_uid(draft_location.mailbox_id, draft_location.uid)
            .unwrap()
            .is_none());
    }

    #[test]
    fn send_later_keeps_the_draft_until_its_time_then_queues_it() {
        let home = TempDataHome::new("later");
        let account = account();
        let draft_location = save_draft(
            &account,
            None,
            &account.address,
            &["anna@example.test".to_owned()],
            &[],
            &["sam@example.test".to_owned()],
            "Later",
            "Hi Anna",
            &[],
            &DraftHeaders::default(),
        )
        .unwrap();
        let draft = Draft {
            to: vec!["anna@example.test".to_owned()],
            bcc: vec!["sam@example.test".to_owned()],
            subject: "Later".to_owned(),
            text: "Hi Anna".to_owned(),
            ..Draft::default()
        };
        let entry = schedule_send(&account, draft, Some(draft_location), 1_000).unwrap();
        let root = home.root.join("lulo/mail");
        let schedule = Schedule::load(&root);
        assert_eq!(schedule.sends, vec![entry.clone()]);
        // Bcc goes in the envelope only, as for an immediate send.
        assert!(entry.envelope_to.contains(&"sam@example.test".to_owned()));
        let storage = MailStorage::open(&root, account.id).unwrap();
        assert_eq!(
            storage
                .outbox_count(rmac_mail_storage::OutboxState::Queued)
                .unwrap(),
            0
        );
        assert!(storage
            .message_by_uid(draft_location.mailbox_id, draft_location.uid)
            .unwrap()
            .is_some());

        let result = send_scheduled(std::slice::from_ref(&account), &entry);
        assert!(matches!(result, DeliveryResult::Queued));
        let storage = MailStorage::open(&root, account.id).unwrap();
        assert_eq!(
            storage
                .outbox_count(rmac_mail_storage::OutboxState::Queued)
                .unwrap(),
            1
        );
        assert!(storage
            .message_by_uid(draft_location.mailbox_id, draft_location.uid)
            .unwrap()
            .is_none());
        assert!(!Schedule::message_path(&root, entry.id).exists());
    }
}
