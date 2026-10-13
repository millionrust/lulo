//! Mail ▸ Settings ▸ Accounts on Windows (ADR 0023 phase 2f).
//!
//! Lulo OS adds accounts in System Settings ▸ Internet Accounts, which
//! Windows does not have, so Mail adds them itself: an address, the servers
//! (found by autoconfig or typed in) and a password, or a browser sign-in for
//! Google and Microsoft. Account details go to Lulo's account store and every
//! secret to the Windows Credential Manager (`rmac_accounts_linux::winstore`).
//! Each step that touches the network runs on its own thread; nothing here
//! polls.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity, FocusHandle,
    FontWeight, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window,
};
use rmac_accounts::autoconfig::{DiscoveryStep, MailConfig, MailServer};
use rmac_accounts::model::{email_domain, Services};
use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts::Secret;
use rmac_accounts_linux::discovery::{discover, SystemDiscovery};
use rmac_accounts_linux::oauth::callback::{register_scheme, OAuthReceiver};
use rmac_accounts_linux::oauth::{OAuthAttempt, SystemOAuthHttp};
use rmac_accounts_linux::winstore::{StoredAccount, WinAccounts};
use rmac_accounts_linux::{Error, GoaApi, OAuthAccount, PasswordMailAccount};
use rmac_editor::InputState;
use rmac_mail_imap::{Authentication, Client, Config, TlsMode};
use rmac_ui::{mac, Button, StyledExt as _, TextField};

type ChangeCallback = Rc<dyn Fn(&mut App)>;

thread_local! {
    static ON_CHANGE: RefCell<Option<ChangeCallback>> = const { RefCell::new(None) };
}

/// Mail's main window registers how to pick up a changed account list.
pub fn set_on_change(callback: impl Fn(&mut App) + 'static) {
    ON_CHANGE.with(|slot| *slot.borrow_mut() = Some(Rc::new(callback)));
}

fn notify_changed(cx: &mut App) {
    if let Some(callback) = ON_CHANGE.with(|slot| slot.borrow().clone()) {
        callback(cx);
    }
}

/// Run `work` on its own thread and await its result from a GPUI task.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> async_channel::Receiver<T> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(work());
    });
    receiver
}

#[derive(Clone)]
enum Stage {
    List,
    Address,
    /// Servers known (found, or typed in below when `manual`), password next.
    Password {
        config: Option<MailConfig>,
        manual: bool,
    },
    /// Waiting for the browser to come back.
    Browser(Provider),
}

pub struct AccountsView {
    stage: Stage,
    accounts: Vec<StoredAccount>,
    address: Entity<InputState>,
    name: Entity<InputState>,
    password: Entity<InputState>,
    imap: Entity<InputState>,
    smtp: Entity<InputState>,
    busy: bool,
    message: Option<SharedString>,
    /// The account whose Remove button was pressed once.
    confirm_remove: Option<String>,
    /// Bumped when a step is abandoned so a late result is ignored.
    generation: u64,
    _focus: FocusHandle,
}

fn friendly(account: &StoredAccount) -> &'static str {
    match account.provider.as_str() {
        "google" => "Google",
        "ms_graph" => "Microsoft",
        _ => "IMAP",
    }
}

impl AccountsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |window: &mut Window, cx: &mut Context<Self>, hint: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let address = input(window, cx, "name@example.com");
        let name = input(window, cx, "Your name");
        let imap = input(window, cx, "imap.example.com");
        let smtp = input(window, cx, "smtp.example.com");
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let mut view = Self {
            stage: Stage::List,
            accounts: Vec::new(),
            address,
            name,
            password,
            imap,
            smtp,
            busy: false,
            message: None,
            confirm_remove: None,
            generation: 0,
            _focus: cx.focus_handle(),
        };
        view.reload();
        view
    }

    fn reload(&mut self) {
        self.accounts = WinAccounts::session()
            .and_then(|store| store.stored())
            .unwrap_or_default()
            .into_iter()
            .filter(|account| account.mail)
            .collect();
    }

    fn text(entity: &Entity<InputState>, cx: &Context<Self>) -> String {
        entity.read(cx).value().to_string()
    }

    fn clear_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.address,
            &self.name,
            &self.password,
            &self.imap,
            &self.smtp,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.stage = Stage::List;
        self.busy = false;
        self.message = None;
        self.generation += 1;
        self.reload();
        cx.notify();
        let app: &mut App = cx;
        notify_changed(app);
    }

    fn fail(&mut self, text: &'static str, stage: Stage, cx: &mut Context<Self>) {
        self.busy = false;
        self.message = Some(text.into());
        self.stage = stage;
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A sign-in waiting in the browser stops waiting.
        if matches!(self.stage, Stage::Browser(_)) {
            let _ = rmac_accounts_linux::oauth::callback::deliver("");
        }
        self.generation += 1;
        self.stage = Stage::List;
        self.busy = false;
        self.message = None;
        self.clear_inputs(window, cx);
        cx.notify();
    }

    /// Address entered: find the servers, or the browser sign-in.
    fn continue_address(&mut self, cx: &mut Context<Self>) {
        let address = Self::text(&self.address, cx).trim().to_owned();
        if email_domain(&address).is_none() || Self::text(&self.name, cx).trim().is_empty() {
            self.message = Some("Enter your name and a valid email address.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.message = None;
        cx.notify();
        let generation = self.generation;
        let found = off_thread(move || discover(&SystemDiscovery, &address));
        cx.spawn(async move |this, cx| {
            let Ok(result) = found.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(discovery) => match discovery.step {
                        DiscoveryStep::OAuth(provider) => this.start_browser(provider, cx),
                        DiscoveryStep::Configured => {
                            this.stage = Stage::Password {
                                config: discovery.config,
                                manual: false,
                            };
                            cx.notify();
                        }
                        _ => this.fail(
                            "Mail couldn't find the servers. Enter the IMAP and SMTP server names.",
                            Stage::Password {
                                config: None,
                                manual: true,
                            },
                            cx,
                        ),
                    },
                    Err(_) => this.fail(
                        "Mail couldn't look up the servers. Enter the IMAP and SMTP server names.",
                        Stage::Password {
                            config: None,
                            manual: true,
                        },
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn start_browser(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.stage = Stage::Browser(provider);
        self.busy = true;
        cx.notify();
        let generation = self.generation;
        let signed_in = off_thread(move || sign_in(provider));
        cx.spawn(async move |this, cx| {
            let Ok(result) = signed_in.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => this.finish(cx),
                    Err(_) => {
                        this.fail("The sign-in didn't finish. Try again.", Stage::Address, cx)
                    }
                }
            });
        })
        .detach();
    }

    /// Password (and servers) entered: check them, then keep the account.
    fn add_password_account(
        &mut self,
        config: Option<MailConfig>,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        let address = Self::text(&self.address, cx).trim().to_owned();
        let name = Self::text(&self.name, cx).trim().to_owned();
        let password = Self::text(&self.password, cx);
        if password.is_empty() {
            self.message = Some("Enter the account's password.".into());
            cx.notify();
            return;
        }
        let config = match config {
            Some(config) if !manual => config,
            _ => {
                let (imap, smtp) = (
                    Self::text(&self.imap, cx).trim().to_owned(),
                    Self::text(&self.smtp, cx).trim().to_owned(),
                );
                if imap.is_empty() || smtp.is_empty() {
                    self.message = Some("Enter both the IMAP and SMTP server names.".into());
                    cx.notify();
                    return;
                }
                manual_config(&address, &imap, &smtp)
            }
        };
        self.busy = true;
        self.message = None;
        cx.notify();
        let generation = self.generation;
        let stage = Stage::Password {
            config: Some(config.clone()),
            manual,
        };
        let saved = off_thread(move || save_password_account(&address, &name, &password, &config));
        cx.spawn(async move |this, cx| {
            let Ok(result) = saved.recv().await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => this.finish(cx),
                    Err(Saved::Rejected) => this.fail(
                        "The server didn't accept that address and password. iCloud and Yahoo need an app-specific password.",
                        stage,
                        cx,
                    ),
                    Err(Saved::Store) => this.fail(
                        "Windows wouldn't keep the account's password.",
                        stage,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn remove(&mut self, path: String, cx: &mut Context<Self>) {
        if self.confirm_remove.as_deref() != Some(path.as_str()) {
            self.confirm_remove = Some(path);
            cx.notify();
            return;
        }
        self.confirm_remove = None;
        let removed =
            off_thread(move || WinAccounts::session().and_then(|store| store.remove(&path)));
        cx.spawn(async move |this, cx| {
            let Ok(result) = removed.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.finish(cx),
                Err(_) => this.fail("The account couldn't be removed.", Stage::List, cx),
            });
        })
        .detach();
    }

    fn field(&self, label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
        div()
            .h_flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .w(px(120.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(label),
            )
            .child(div().flex_1().child(TextField::new(input)))
    }

    fn buttons(
        &self,
        primary: &'static str,
        cx: &mut Context<Self>,
        go: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let busy = self.busy;
        div()
            .h_flex()
            .gap(px(8.0))
            .child(
                Button::new("mail-account-cancel", "Cancel")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            )
            .child(
                Button::new("mail-account-go", if busy { "Working…" } else { primary })
                    .small()
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| go(this, cx))),
            )
    }
}

impl Render for AccountsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = self.message.clone().map(|text| {
            div()
                .text_size(px(12.0))
                .text_color(mac::text_secondary())
                .child(text)
        });
        let body = match self.stage.clone() {
            Stage::List => {
                let rows = self.accounts.iter().map(|account| {
                    let path = account.path();
                    let confirming = self.confirm_remove.as_deref() == Some(path.as_str());
                    div()
                        .p(px(8.0))
                        .rounded(px(mac::radius_card()))
                        .bg(mac::control_fill())
                        .h_flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(13.0))
                                .child(account.identity.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(mac::text_secondary())
                                .child(friendly(account)),
                        )
                        .child(
                            Button::new(SharedString::from(format!("mail-remove-{path}")), if confirming { "Confirm Remove" } else { "Remove" })
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| this.remove(path.clone(), cx))),
                        )
                });
                div()
                    .v_flex()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_color(mac::text_secondary())
                            .text_size(px(12.0))
                            .child(
                                "Mail keeps each account's details here and its password in the Windows Credential Manager.",
                            ),
                    )
                    .when(self.accounts.is_empty(), |parent| {
                        parent.child(
                            div()
                                .text_color(mac::text_secondary())
                                .text_size(px(12.0))
                                .child("No mail accounts yet."),
                        )
                    })
                    .child(div().v_flex().gap(px(4.0)).children(rows))
                    .child(
                        Button::new("mail-account-add", "Add Account…")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stage = Stage::Address;
                                this.message = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
            Stage::Address => div()
                .v_flex()
                .gap(px(10.0))
                .child(self.field("Name", &self.name))
                .child(self.field("Email address", &self.address))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(mac::text_secondary())
                        .child("Gmail and Outlook.com sign in with your browser. iCloud and Yahoo need an app-specific password."),
                )
                .child(self.buttons("Continue", cx, |this, cx| this.continue_address(cx)))
                .into_any_element(),
            Stage::Password { config, manual } => {
                let shown = config.clone();
                div()
                    .v_flex()
                    .gap(px(10.0))
                    .child(self.field("Email address", &self.address))
                    .when(manual, |parent| {
                        parent
                            .child(self.field("IMAP server", &self.imap))
                            .child(self.field("SMTP server", &self.smtp))
                    })
                    .child(self.field("Password", &self.password))
                    .when_some(shown.filter(|_| !manual), |parent, config| {
                        parent.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(mac::text_secondary())
                                .child(format!("Servers: {} and {}", config.imap.host, config.smtp.host)),
                        )
                    })
                    .child(self.buttons("Add", cx, move |this, cx| {
                        this.add_password_account(config.clone(), manual, cx)
                    }))
                    .into_any_element()
            }
            Stage::Browser(provider) => div()
                .v_flex()
                .gap(px(10.0))
                .child(div().text_size(px(13.0)).child(format!(
                    "Finish signing in to {} in your browser, then come back to Mail.",
                    if provider == Provider::Google { "Google" } else { "Microsoft" }
                )))
                .child(self.buttons("Waiting…", cx, |_, _| {}))
                .into_any_element(),
        };
        div().v_flex().gap(px(8.0)).child(body).children(message)
    }
}

enum Saved {
    Rejected,
    Store,
}

fn manual_config(address: &str, imap: &str, smtp: &str) -> MailConfig {
    MailConfig {
        imap: MailServer {
            host: imap.into(),
            port: 993,
            security: SocketSecurity::Tls,
            username: address.into(),
        },
        smtp: MailServer {
            host: smtp.into(),
            port: 587,
            security: SocketSecurity::StartTls,
            username: address.into(),
        },
    }
}

/// Sign in to the IMAP server with the password, then keep the account.
fn save_password_account(
    address: &str,
    name: &str,
    password: &str,
    config: &MailConfig,
) -> Result<(), Saved> {
    let tls = match config.imap.security {
        SocketSecurity::Tls => TlsMode::Implicit,
        SocketSecurity::StartTls => TlsMode::StartTls,
    };
    let mut client = Client::connect(&Config {
        host: config.imap.host.clone(),
        port: config.imap.port,
        tls,
        timeout: Duration::from_secs(20),
    })
    .map_err(|_| Saved::Rejected)?;
    let secret = rmac_mail_imap::Secret::new(password);
    client
        .authenticate(Authentication::Plain {
            user: &config.imap.username,
            password: &secret,
        })
        .map_err(|_| Saved::Rejected)?;
    let password = Secret::new(password.to_owned());
    WinAccounts::session()
        .and_then(|store| {
            store.add_password_mail(&PasswordMailAccount {
                address,
                display_name: name,
                config,
                imap_password: &password,
                smtp_password: &password,
            })
        })
        .map(|_| ())
        .map_err(|_| Saved::Store)
}

/// The browser sign-in: register the redirect scheme for Mail, open the
/// provider's page, wait for the redirect, trade the code for tokens and keep
/// the account.
fn sign_in(provider: Provider) -> Result<(), Error> {
    let attempt = OAuthAttempt::new(provider)?;
    let receiver = OAuthReceiver::begin()?;
    let scheme = attempt.redirect_scheme().ok_or(Error::InvalidResponse)?;
    let exe = std::env::current_exe().map_err(|_| Error::Unavailable)?;
    register_scheme(&scheme, &exe)?;
    async_io::block_on(attempt.open_in_browser())?;
    let (client, uri) = receiver.receive_for(&attempt)?;
    let tokens = attempt.complete(&client, &uri, &SystemOAuthHttp)?;
    WinAccounts::session()?.add_oauth(&OAuthAccount {
        provider,
        identity: &tokens.identity,
        presentation_identity: &tokens.presentation_identity,
        access_token: &tokens.access_token,
        refresh_token: tokens.refresh_token.as_ref(),
        expires_at: tokens.expires_at,
        services: Services::MAIL_ONLY,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_in_servers_use_the_standard_secure_ports() {
        let config = manual_config("ada@example.com", "imap.example.com", "smtp.example.com");
        assert_eq!(config.imap.port, 993);
        assert_eq!(config.imap.security, SocketSecurity::Tls);
        assert_eq!(config.smtp.port, 587);
        assert_eq!(config.smtp.security, SocketSecurity::StartTls);
        assert_eq!(config.smtp.username, "ada@example.com");
    }

    #[test]
    fn an_unreachable_server_is_rejected_not_stored() {
        // Port 1 on loopback refuses the connection at once.
        let config = MailConfig {
            imap: MailServer {
                host: "127.0.0.1".into(),
                port: 1,
                security: SocketSecurity::Tls,
                username: "ada@example.com".into(),
            },
            smtp: MailServer {
                host: "127.0.0.1".into(),
                port: 1,
                security: SocketSecurity::StartTls,
                username: "ada@example.com".into(),
            },
        };
        assert!(matches!(
            save_password_account("ada@example.com", "Ada", "pw", &config),
            Err(Saved::Rejected)
        ));
    }
}
