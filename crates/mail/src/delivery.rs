//! Account discovery and outbound delivery. All calls in this module block and
//! must run outside the GPUI thread.

use std::path::PathBuf;

use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts_linux::{goa::GoaBus, GoaApi};
use rmac_mail_mime::{build, Draft};
use rmac_mail_runtime::account_id;
use rmac_mail_smtp::{drain_outbox, Authentication, Config, Secret, Security};
use rmac_mail_storage::MailStorage;
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

pub fn deliver(account: &ComposeAccount, mut draft: Draft) -> DeliveryResult {
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
