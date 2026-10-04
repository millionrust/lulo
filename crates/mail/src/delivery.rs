//! Account discovery and outbound delivery. All calls in this module block and
//! must run outside the GPUI thread.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts_linux::{goa::GoaBus, GoaApi};
use rmac_mail_mime::{build, guess_content_type, Draft};
use rmac_mail_runtime::account_id;
use rmac_mail_smtp::{drain_outbox, Authentication, Config, Secret, Security};
use rmac_mail_storage::{MailStorage, NewMessage, FLAG_DRAFT};
use uuid::Uuid;

#[derive(Clone)]
pub struct ComposeAccount {
    pub path: String,
    pub id: Uuid,
    pub address: String,
    pub provider: String,
}

/// Startup discovery runs before GPUI. No credential is retained in the UI.
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

fn data_root() -> Option<PathBuf> {
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
    let recipients = to
        .iter()
        .chain(cc)
        .chain(bcc)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let preview: String = body.chars().filter(|c| *c != '\n').take(160).collect();
    let subject = if subject.trim().is_empty() {
        "(No Subject)"
    } else {
        subject
    };
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
            preview: &preview,
            received_at: now_unix(),
            flags: FLAG_DRAFT,
            body: None,
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
    let Some(root) = data_root() else {
        return DeliveryResult::Failed("Mail could not find its data folder");
    };
    let mut storage = match MailStorage::open(&root, account.id) {
        Ok(storage) => storage,
        Err(_) => return DeliveryResult::Failed("Mail could not open the Outbox"),
    };
    if storage
        .queue_outbox(&built.envelope_from, &built.envelope_to, &built.bytes)
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
        return DeliveryResult::Queued;
    }
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
    let Some(server) = provider.info().servers else {
        return DeliveryResult::Queued;
    };
    let goa = match GoaBus::session() {
        Ok(goa) => goa,
        Err(_) => return DeliveryResult::Queued,
    };
    let auth = if account.provider == "google" {
        match goa.access_token(&account.path) {
            Ok(token) => Authentication::Xoauth2 {
                user: account.address.clone(),
                token: Secret::new(token.expose().to_owned()),
            },
            Err(_) => return DeliveryResult::Queued,
        }
    } else {
        match goa.password(&account.path, "smtp-password") {
            Ok(password) => Authentication::Plain {
                user: account.address.clone(),
                password: Secret::new(password.expose().to_owned()),
            },
            Err(_) => return DeliveryResult::Queued,
        }
    };
    let config = Config {
        host: server.smtp_host.into(),
        port: server.smtp_port,
        helo_name: "localhost".into(),
        security: match server.smtp_security {
            SocketSecurity::Tls => Security::ImplicitTls,
            SocketSecurity::StartTls => Security::StartTls,
        },
    };
    match drain_outbox(&mut storage, &config, &auth) {
        Ok(_) => DeliveryResult::Sent,
        Err(_) => DeliveryResult::Queued,
    }
}
