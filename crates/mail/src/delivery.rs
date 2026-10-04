//! Account discovery and outbound delivery. All calls in this module block and
//! must run outside the GPUI thread.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_accounts::provider::{Provider, SocketSecurity};
#[cfg(target_os = "linux")]
use rmac_accounts_linux::{goa::GoaBus, GoaApi};
use rmac_mail_mime::{build, guess_content_type, Draft};
#[cfg(target_os = "linux")]
use rmac_mail_runtime::account_id;
#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
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
    let recipients = to.iter().chain(bcc).cloned().collect::<Vec<_>>().join(", ");
    let cc_joined = cc.join(", ");
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
            cc: &cc_joined,
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

#[cfg(not(target_os = "linux"))]
fn smtp_authentication(_account: &ComposeAccount) -> Option<Authentication> {
    None
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
    let Some(auth) = smtp_authentication(account) else {
        return DeliveryResult::Queued;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_mail_mime::Draft;

    /// This test is the only one in the `rmac-mail` binary target reading
    /// `XDG_DATA_HOME` (`data_root`'s path), so mutating it process-wide
    /// for its duration is safe: nothing else in this test binary runs
    /// concurrently against the real value.
    struct TempDataHome {
        previous: Option<std::ffi::OsString>,
        root: PathBuf,
    }
    impl TempDataHome {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rmac-mail-delivery-test-{tag}-{}-{}",
                std::process::id(),
                tag.len()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let previous = std::env::var_os("XDG_DATA_HOME");
            std::env::set_var("XDG_DATA_HOME", &root);
            Self { previous, root }
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
        assert!(matches!(result, DeliveryResult::Queued));

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
}
