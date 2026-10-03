//! Linux account, connectivity, notification and Dock signal adapters.

use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts_linux::{goa::GoaBus, AccountChange, GoaAccount, GoaApi};
use rmac_mail_imap::TlsMode;
use uuid::Uuid;
use zbus::{
    blocking::{Connection, MessageIterator, Proxy},
    zvariant::Value,
};

use crate::imap::ImapAuth;
use crate::{
    account_id, Account, Backend, BackendFactory, Error, EventSink, ImapFactory, ImapSettings,
    NewMail, Runtime, Snapshot, Transport,
};

/// The GOA account list is authoritative. Other IMAP accounts may provide
/// manual server settings through `Runtime::upsert_account` until ACC-3 exposes
/// those details in its account model.
pub fn resolve_goa(account: &GoaAccount) -> Option<Account> {
    if !account.services.mail {
        return None;
    }
    let transport = match account.provider.as_str() {
        "ms_graph" => Transport::Graph,
        "google" | "imap_smtp" => {
            let provider = if account.provider == "google" {
                Provider::Google
            } else {
                let domain = account.identity.rsplit_once('@')?.1;
                Provider::from_domain(domain)
            };
            let preset = provider.info().servers?;
            Transport::Imap(ImapSettings {
                host: preset.imap_host.into(),
                port: preset.imap_port,
                tls: match preset.imap_security {
                    SocketSecurity::Tls => TlsMode::Implicit,
                    SocketSecurity::StartTls => TlsMode::StartTls,
                },
                user: account.identity.clone(),
                auth: if provider == Provider::Google {
                    ImapAuth::XOAuth2
                } else {
                    ImapAuth::Password
                },
            })
        }
        _ => return None,
    };
    Some(Account {
        path: account.path.clone(),
        id: account_id(account),
        address: account.identity.clone(),
        transport,
    })
}

/// Starts the signal watch after Mail opens. The blocking GOA API never runs
/// on the GPUI thread. The thread dies with the Mail process.
pub fn watch_goa(runtime: Arc<Runtime>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut delay = Duration::from_secs(1);
        loop {
            if let Ok(goa) = GoaBus::session() {
                let result = goa.watch(&mut |change| match change {
                    AccountChange::Added(account) | AccountChange::Updated(account) => {
                        if let Some(resolved) = resolve_goa(&account) {
                            runtime.upsert_account(resolved);
                        } else {
                            runtime.remove_account(&account.path);
                        }
                    }
                    AccountChange::Removed(path) => runtime.remove_account(&path),
                });
                if result.is_ok() {
                    delay = Duration::from_secs(1);
                }
            }
            thread::sleep(delay);
            delay = super::next_backoff(delay);
        }
    })
}

/// NetworkManager's connectivity property and change signals replace any
/// periodic connectivity probe. Reconnect wakes account workers immediately.
pub fn watch_connectivity(runtime: Arc<Runtime>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut delay = Duration::from_secs(1);
        loop {
            if let Ok(connection) = Connection::system() {
                let rule = "type='signal',sender='org.freedesktop.NetworkManager',path='/org/freedesktop/NetworkManager',interface='org.freedesktop.DBus.Properties'";
                if let Ok(signals) = MessageIterator::for_match_rule(rule, &connection, Some(16)) {
                    let mut last = None;
                    let mut update = || {
                        if let Ok(proxy) = Proxy::new(
                            &connection,
                            "org.freedesktop.NetworkManager",
                            "/org/freedesktop/NetworkManager",
                            "org.freedesktop.NetworkManager",
                        ) {
                            if let Ok(value) = proxy.get_property::<u32>("Connectivity") {
                                let online = value >= 3;
                                if last != Some(online) {
                                    runtime.set_online(online);
                                    last = Some(online);
                                }
                            }
                        }
                    };
                    update();
                    delay = Duration::from_secs(1);
                    for signal in signals {
                        if signal.is_err() {
                            break;
                        }
                        update();
                    }
                }
            }
            thread::sleep(delay);
            delay = super::next_backoff(delay);
        }
    })
}

pub enum UiEvent {
    Snapshot(Snapshot),
    NewMail(NewMail),
    Failure { account: Uuid, message: String },
    OpenMessage { account: Uuid, message_id: i64 },
}

/// Posts to the same freedesktop service owned by Lulo Notification Centre.
/// The optional channel gives the Mail window immutable state updates.
pub struct DesktopSink {
    unread: Mutex<HashMap<Uuid, i64>>,
    ui: Option<mpsc::Sender<UiEvent>>,
    notification_targets: Arc<Mutex<HashMap<u32, (Uuid, i64)>>>,
}

impl DesktopSink {
    pub fn new(ui: Option<mpsc::Sender<UiEvent>>) -> Self {
        let notification_targets = Arc::new(Mutex::new(HashMap::new()));
        if let Some(sender) = ui.clone() {
            let targets = Arc::clone(&notification_targets);
            thread::spawn(move || watch_notification_actions(sender, targets));
        }
        Self {
            unread: Mutex::new(HashMap::new()),
            ui,
            notification_targets,
        }
    }
}

fn watch_notification_actions(
    ui: mpsc::Sender<UiEvent>,
    targets: Arc<Mutex<HashMap<u32, (Uuid, i64)>>>,
) {
    let Ok(connection) = Connection::session() else {
        return;
    };
    let rule = "type='signal',sender='org.freedesktop.Notifications',path='/org/freedesktop/Notifications',interface='org.freedesktop.Notifications',member='ActionInvoked'";
    let Ok(signals) = MessageIterator::for_match_rule(rule, &connection, Some(64)) else {
        return;
    };
    for signal in signals {
        let Ok(signal) = signal else {
            break;
        };
        let Ok((id, action)) = signal.body().deserialize::<(u32, String)>() else {
            continue;
        };
        if action != "default" {
            continue;
        }
        let target = targets
            .lock()
            .expect("mail notification lock poisoned")
            .remove(&id);
        if let Some((account, message_id)) = target {
            if ui
                .send(UiEvent::OpenMessage {
                    account,
                    message_id,
                })
                .is_err()
            {
                break;
            }
        }
    }
}

impl EventSink for DesktopSink {
    fn failure(&self, account: Uuid, error: &Error) {
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::Failure {
                account,
                message: error.to_string(),
            });
        }
    }
    fn snapshot(&self, value: Snapshot) {
        let count = {
            let mut unread = self.unread.lock().expect("mail unread lock poisoned");
            unread.insert(value.account, value.unread_inbox);
            unread.values().copied().sum::<i64>()
        };
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::Snapshot(value));
        }
        if let Ok(connection) = Connection::session() {
            let mut properties = HashMap::new();
            properties.insert("count", Value::I64(count));
            properties.insert("count-visible", Value::Bool(count > 0));
            let _ = connection.emit_signal(
                None::<&str>,
                "/com/canonical/Unity/LauncherEntry",
                "com.canonical.Unity.LauncherEntry",
                "Update",
                &("application://org.rmac.Mail.desktop", properties),
            );
        }
    }

    fn new_mail(&self, value: NewMail) {
        if let Some(ui) = &self.ui {
            let _ = ui.send(UiEvent::NewMail(value.clone()));
        }
        if let Ok(connection) = Connection::session() {
            let mut hints = HashMap::new();
            hints.insert("desktop-entry", Value::Str("org.rmac.Mail".into()));
            let body = format!("{}\n{}", value.subject, value.preview);
            let reply = connection.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Mail",
                    0_u32,
                    "org.rmac.Mail",
                    value.sender,
                    body,
                    vec!["default", "Open"],
                    hints,
                    -1_i32,
                ),
            );
            if let Ok(reply) = reply {
                if let Ok(id) = reply.body().deserialize::<u32>() {
                    let mut targets = self
                        .notification_targets
                        .lock()
                        .expect("mail notification lock poisoned");
                    if targets.len() >= 1024 {
                        if let Some(oldest) = targets.keys().next().copied() {
                            targets.remove(&oldest);
                        }
                    }
                    targets.insert(id, (value.account, value.message_id));
                }
            }
        }
    }
}

/// Graph sync is supplied by MAIL-9. The runtime owns its five-minute
/// schedule; the adapter only needs to implement `BackendFactory`.
pub struct ProviderFactory {
    pub imap: ImapFactory,
    pub graph: Arc<dyn BackendFactory>,
}

impl BackendFactory for ProviderFactory {
    fn connect(&self, account: &Account) -> Result<Box<dyn Backend>, Error> {
        match &account.transport {
            Transport::Imap(_) => self.imap.connect(account),
            Transport::Graph => self.graph.connect(account),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_accounts::model::Services;

    fn account(provider: &str, address: &str) -> GoaAccount {
        GoaAccount {
            path: "/org/gnome/OnlineAccounts/Accounts/1".into(),
            id: "1".into(),
            provider: provider.into(),
            identity: address.into(),
            services: Services {
                mail: true,
                calendar: false,
                contacts: false,
            },
        }
    }

    #[test]
    fn goa_discovery_routes_microsoft_to_graph_and_google_to_idle() {
        assert!(matches!(
            resolve_goa(&account("ms_graph", "a@outlook.com"))
                .unwrap()
                .transport,
            Transport::Graph
        ));
        let google = resolve_goa(&account("google", "a@gmail.com")).unwrap();
        assert!(matches!(
            google.transport,
            Transport::Imap(ImapSettings {
                auth: ImapAuth::XOAuth2,
                ..
            })
        ));
        assert!(resolve_goa(&account("imap_smtp", "a@unknown.example")).is_none());
    }
}
