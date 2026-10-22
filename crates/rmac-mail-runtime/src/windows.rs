//! Windows account and notification adapters (ADR 0023 phase 2f): Lulo's own
//! account store (`rmac_accounts_linux::winstore`, secrets in the Credential
//! Manager) in place of GNOME Online Accounts, and toasts in place of the
//! freedesktop notification service.
//!
//! There is no connectivity watcher here: accounts start online, and a sync
//! that cannot reach the server backs off and retries on its own schedule, so
//! nothing polls while Mail is idle (IMAP waits in IDLE; Graph on its
//! five-minute deadline).

use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
};

use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts_linux::winstore::{Kind, StoredAccount, WinAccounts};
use rmac_accounts_linux::GoaApi;
use rmac_mail_imap::TlsMode;
use uuid::Uuid;

use crate::imap::ImapAuth;
use crate::{
    account_id, Account, Error, EventSink, ImapFactory, ImapSettings, NewMail, Runtime, Snapshot,
    Transport, UiEvent,
};

/// The AppUserModelID Mail's Start Menu shortcut carries
/// (`rmac_ui::app_menu::app_user_model_id`).
const AUMID: &str = "Lulo.Mail";

/// The sync account for a stored one, or `None` when it has no mail service
/// or no usable servers.
pub fn resolve(stored: &StoredAccount) -> Option<Account> {
    if !stored.mail {
        return None;
    }
    let transport = match (stored.provider.as_str(), stored.kind) {
        ("ms_graph", Kind::OAuth) => Transport::Graph,
        ("google", Kind::OAuth) => {
            let preset = Provider::Google.info().servers?;
            Transport::Imap(ImapSettings {
                host: preset.imap_host.into(),
                port: preset.imap_port,
                tls: tls(preset.imap_security),
                user: stored.identity.clone(),
                auth: ImapAuth::XOAuth2,
            })
        }
        ("imap_smtp", Kind::Password) => {
            let imap = stored.imap.as_ref()?;
            Transport::Imap(ImapSettings {
                host: imap.host.clone(),
                port: imap.port,
                tls: tls(imap.security()),
                user: imap.username.clone(),
                auth: ImapAuth::Password,
            })
        }
        _ => return None,
    };
    Some(Account {
        path: stored.path(),
        id: account_id(&stored.to_goa()),
        address: stored.identity.clone(),
        transport,
    })
}

fn tls(security: SocketSecurity) -> TlsMode {
    match security {
        SocketSecurity::Tls => TlsMode::Implicit,
        SocketSecurity::StartTls => TlsMode::StartTls,
    }
}

/// Start a sync worker for every mail account in the store. Blocking (reads
/// the account index); run it off the UI thread.
pub fn seed_accounts(runtime: &Runtime) {
    if let Ok(accounts) = WinAccounts::session().and_then(|store| store.stored()) {
        for account in accounts.iter().filter_map(resolve) {
            runtime.upsert_account(account);
        }
    }
}

/// IMAP connections whose password or OAuth token comes from the store.
pub fn imap_factory() -> ImapFactory {
    ImapFactory::new(Arc::new(|account, auth| {
        let store = WinAccounts::session().map_err(|_| Error::Account)?;
        let value = match auth {
            ImapAuth::XOAuth2 => store.access_token(&account.path),
            ImapAuth::Password => store.password(&account.path, "imap-password"),
        }
        .map_err(|_| Error::Account)?;
        Ok(rmac_mail_imap::Secret::new(value.expose()))
    }))
}

/// Mail's side of a toast: new messages as toasts, everything else to the
/// window through the same events Linux sends.
pub struct ToastSink {
    ui: Option<mpsc::Sender<UiEvent>>,
    unread: Mutex<HashMap<Uuid, i64>>,
}

impl ToastSink {
    pub fn new(ui: Option<mpsc::Sender<UiEvent>>) -> Self {
        Self {
            ui,
            unread: Mutex::new(HashMap::new()),
        }
    }

    /// Unread messages across every account: Mail's Dock badge.
    pub fn unread_total(&self) -> i64 {
        self.unread
            .lock()
            .expect("mail unread lock poisoned")
            .values()
            .sum()
    }
}

/// Mail's unread count on its Dock tile, as on the Mac and Lulo OS (where
/// it is a LauncherEntry signal), off the caller's thread.
fn publish_badge(count: i64) {
    let _ = std::thread::Builder::new()
        .name("mail-badge".into())
        .spawn(move || {
            rmac_notifications::pipe::post_badge(&rmac_notifications::pipe::Badge {
                count,
                progress: None,
            })
        });
}

impl EventSink for ToastSink {
    fn removed(&self, account: Uuid) {
        self.unread
            .lock()
            .expect("mail unread lock poisoned")
            .remove(&account);
        publish_badge(self.unread_total());
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::AccountRemoved(account));
        }
    }

    fn failure(&self, account: Uuid, error: &Error) {
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::Failure {
                account,
                message: error.to_string(),
            });
        }
    }

    fn snapshot(&self, value: Snapshot) {
        self.unread
            .lock()
            .expect("mail unread lock poisoned")
            .insert(value.account, value.unread_inbox);
        publish_badge(self.unread_total());
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::Snapshot(value));
        }
    }

    fn new_mail(&self, value: NewMail) {
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::NewMail(value.clone()));
        }
        if !crate::alerts::notify() {
            return;
        }
        let body = format!("{}\n{}", value.subject, value.preview);
        let _ = rmac_win_toast::show(AUMID, &value.sender, &body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_accounts_linux::winstore::StoredServer;

    fn stored(provider: &str, kind: Kind) -> StoredAccount {
        StoredAccount {
            id: "abc".into(),
            provider: provider.into(),
            kind,
            identity: "ada@example.com".into(),
            display_name: "Ada".into(),
            mail: true,
            calendar: false,
            contacts: false,
            imap: Some(StoredServer {
                host: "imap.example.com".into(),
                port: 993,
                security: "tls".into(),
                username: "ada@example.com".into(),
            }),
            smtp: None,
            caldav_uri: None,
            expires_at: 0,
        }
    }

    #[test]
    fn a_password_account_uses_its_own_servers() {
        let account = resolve(&stored("imap_smtp", Kind::Password)).unwrap();
        assert_eq!(account.path, "win:abc");
        let Transport::Imap(settings) = account.transport else {
            panic!("expected IMAP");
        };
        assert_eq!(settings.host, "imap.example.com");
        assert_eq!(settings.auth, ImapAuth::Password);
        assert_eq!(settings.tls, TlsMode::Implicit);
    }

    #[test]
    fn google_uses_oauth_on_the_preset_and_microsoft_uses_graph() {
        let google = resolve(&stored("google", Kind::OAuth)).unwrap();
        let Transport::Imap(settings) = google.transport else {
            panic!("expected IMAP");
        };
        assert_eq!(settings.host, "imap.gmail.com");
        assert_eq!(settings.auth, ImapAuth::XOAuth2);
        let microsoft = resolve(&stored("ms_graph", Kind::OAuth)).unwrap();
        assert_eq!(microsoft.transport, Transport::Graph);
    }

    #[test]
    fn accounts_without_mail_or_with_odd_shapes_are_left_out() {
        let mut no_mail = stored("imap_smtp", Kind::Password);
        no_mail.mail = false;
        assert!(resolve(&no_mail).is_none());
        assert!(resolve(&stored("caldav", Kind::Password)).is_none());
        assert!(resolve(&stored("ms_graph", Kind::Password)).is_none());
    }
}
