//! Internet Accounts: GOA is read and mutated only on blocking workers.

use super::*;
mod render;
#[cfg(target_os = "linux")]
use rmac_accounts::autoconfig::MailServer;
use rmac_accounts::autoconfig::{DiscoveryStep, MailConfig};
use rmac_accounts::model::{Service, Services};
#[cfg(target_os = "linux")]
use rmac_accounts::provider::SocketSecurity;
use rmac_accounts::Secret;
use rmac_accounts_linux::discovery::{discover, SystemDiscovery};
use rmac_accounts_linux::oauth::Tokens;
#[cfg(target_os = "linux")]
use rmac_accounts_linux::oauth::{OAuthAttempt, SystemOAuthHttp};
use rmac_accounts_linux::GoaAccount;
#[cfg(target_os = "linux")]
use rmac_accounts_linux::{GoaApi, OAuthAccount, PasswordCalendarAccount, PasswordMailAccount};
use rmac_accounts_ui::{AddSheet, Choice, Step};

pub(super) struct Sheet {
    pub(super) model: AddSheet,
    pub(super) address: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) caldav: Entity<InputState>,
    pub(super) imap: Entity<InputState>,
    pub(super) smtp: Entity<InputState>,
    pub(super) tokens: Option<Tokens>,
    pub(super) manual: bool,
    pub(super) discovered_config: Option<MailConfig>,
    pub(super) generation: u64,
    pub(super) cancellation: std::sync::Arc<OAuthCancellation>,
}

static NEXT_SHEET_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Default)]
pub(super) struct OAuthCancellation {
    cancelled: std::sync::atomic::AtomicBool,
    sender: std::sync::Mutex<Option<std::sync::mpsc::Sender<(String, String)>>>,
}

impl OAuthCancellation {
    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(sender) = self.sender.lock().unwrap().take() {
            let _ = sender.send((String::new(), String::new()));
        }
    }
}

fn account_field(
    window: &mut Window,
    cx: &mut Context<Settings>,
    hint: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(hint))
}

fn account_error(error: rmac_accounts_linux::Error) -> &'static str {
    use rmac_accounts_linux::Error;
    match error {
        Error::Unavailable => "Internet Accounts is unavailable. Check that GNOME Online Accounts is installed and try again.",
        Error::Busy => "Finish signing in in the other window, then try again.",
        Error::Network => "Lulo couldn't reach the account provider. Check your connection and try again.",
        Error::InvalidResponse | Error::SignInFailed => "Lulo couldn't sign in. Check your details and try again.",
    }
}

#[cfg(target_os = "linux")]
fn read_accounts() -> Result<Vec<GoaAccount>, rmac_accounts_linux::Error> {
    rmac_accounts_linux::goa::GoaBus::session()?.accounts()
}

#[cfg(not(target_os = "linux"))]
fn read_accounts() -> Result<Vec<GoaAccount>, rmac_accounts_linux::Error> {
    Err(rmac_accounts_linux::Error::Unavailable)
}

impl Settings {
    fn open_account_password_help(&mut self, choice: Choice, cx: &mut Context<Self>) {
        let Some(url) = choice.password_help_url() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_portal::open_uri(url).await;
            if result.is_err() {
                let _ = this.update(cx, |this: &mut Settings, cx| {
                    if let Some(sheet) = this.internet_account_sheet.as_mut() {
                        sheet.model.error =
                            Some("Lulo couldn't open the account provider's website.");
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub(super) fn refresh_internet_accounts(&mut self, cx: &mut Context<Self>) {
        self.internet_accounts_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(read_accounts).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.internet_accounts_loading = false;
                match result {
                    Ok(accounts) => {
                        this.internet_accounts = accounts;
                        this.internet_accounts_error = None;
                    }
                    Err(error) => this.internet_accounts_error = Some(account_error(error).into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn watch_internet_accounts(cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        {
            let (sender, receiver) = async_channel::bounded(1);
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while receiver.recv().await.is_ok() {
                    let _ = this.update(cx, |this: &mut Settings, cx| {
                        this.refresh_internet_accounts(cx)
                    });
                }
            })
            .detach();
            cx.spawn(async move |_, _| {
                blocking::unblock(move || {
                    if let Ok(bus) = rmac_accounts_linux::goa::GoaBus::session() {
                        let _ = bus.watch(&mut |_| {
                            let _ = sender.try_send(());
                        });
                    }
                })
                .await;
            })
            .detach();
        }
    }

    fn open_account_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.internet_account_sheet = Some(Sheet {
            model: AddSheet::default(),
            address: account_field(window, cx, "name@example.com"),
            name: account_field(window, cx, "Your name"),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Password")
                    .masked(true)
            }),
            caldav: account_field(window, cx, "https://calendar.example.com/"),
            imap: account_field(window, cx, "imap.example.com"),
            smtp: account_field(window, cx, "smtp.example.com"),
            tokens: None,
            manual: false,
            discovered_config: None,
            generation: NEXT_SHEET_ID.fetch_add(10, std::sync::atomic::Ordering::Relaxed),
            cancellation: std::sync::Arc::default(),
        });
        self.internet_accounts_error = None;
        cx.notify();
    }

    fn close_account_sheet(&mut self, cx: &mut Context<Self>) {
        if let Some(sheet) = &self.internet_account_sheet {
            sheet.cancellation.cancel();
        }
        self.internet_account_sheet = None;
        self.internet_accounts_busy = false;
        cx.notify();
    }

    fn continue_account_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.internet_account_sheet.as_mut() else {
            return;
        };
        sheet.model.address = sheet.address.read(cx).value().to_string();
        if !sheet.model.continue_choice() {
            cx.notify();
            return;
        }
        let oauth = sheet.model.step == Step::Browser;
        let discover =
            sheet.model.choice == Some(Choice::OtherMail) && !sheet.model.address.is_empty();
        if discover {
            sheet.model.step = Step::Discovering;
        }
        cx.notify();
        if discover {
            self.start_account_discovery(cx);
        } else if oauth {
            self.start_account_oauth(cx);
        }
    }

    fn start_account_discovery(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.internet_account_sheet.as_mut() else {
            return;
        };
        let address = sheet.model.address.clone();
        sheet.generation += 1;
        let generation = sheet.generation;
        self.internet_accounts_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || discover(&SystemDiscovery, &address)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let Some(sheet) = this.internet_account_sheet.as_mut().filter(|sheet| sheet.generation == generation) else { return; };
                this.internet_accounts_busy = false;
                match result {
                    Ok(discovery) => match discovery.step {
                        DiscoveryStep::OAuth(provider) => {
                            sheet.model.choice = Some(match provider {
                                rmac_accounts::provider::Provider::Google => Choice::Google,
                                rmac_accounts::provider::Provider::Microsoft => Choice::Microsoft,
                                _ => Choice::OtherMail,
                            });
                            sheet.model.step = Step::Browser;
                            this.start_account_oauth(cx);
                        }
                        DiscoveryStep::Configured => {
                            sheet.discovered_config = discovery.config;
                            sheet.model.step = Step::Credentials;
                        }
                        _ => {
                            sheet.manual = true;
                            sheet.model.step = Step::Credentials;
                            sheet.model.error = Some("Lulo couldn't find the mail servers. Enter the IMAP and SMTP server names.");
                        }
                    },
                    Err(error) => {
                        sheet.manual = true;
                        sheet.model.step = Step::Credentials;
                        sheet.model.error = Some(account_error(error));
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn start_account_oauth(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.internet_account_sheet.as_mut() else {
            return;
        };
        let Some(choice) = sheet.model.choice else {
            return;
        };
        let provider = choice.provider();
        #[cfg(not(target_os = "linux"))]
        let _ = provider;
        sheet.generation += 1;
        let generation = sheet.generation;
        #[cfg(target_os = "linux")]
        let cancellation = sheet.cancellation.clone();
        self.internet_accounts_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            #[cfg(target_os = "linux")]
            let result = async {
                let (attempt, receiver) = blocking::unblock(move || {
                    let attempt = OAuthAttempt::new(provider)?;
                    let receiver = rmac_accounts_linux::oauth::callback::OAuthReceiver::begin()?;
                    Ok::<_, rmac_accounts_linux::Error>((attempt, receiver))
                })
                .await?;
                *cancellation.sender.lock().unwrap() = Some(receiver.cancel_sender());
                if cancellation
                    .cancelled
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    return Err(rmac_accounts_linux::Error::Busy);
                }
                attempt.open_in_browser().await?;
                blocking::unblock(move || {
                    let (client, uri) = receiver.receive_for(&attempt)?;
                    attempt.complete(&client, &uri, &SystemOAuthHttp)
                })
                .await
            }
            .await;
            #[cfg(not(target_os = "linux"))]
            let result: Result<Tokens, rmac_accounts_linux::Error> =
                Err(rmac_accounts_linux::Error::Unavailable);
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(sheet) = this
                    .internet_account_sheet
                    .as_mut()
                    .filter(|sheet| sheet.generation == generation)
                {
                    this.internet_accounts_busy = false;
                    match result {
                        Ok(tokens) => {
                            sheet.model.address = tokens.presentation_identity.clone();
                            sheet.tokens = Some(tokens);
                            sheet.model.step = Step::Services;
                        }
                        Err(error) => {
                            sheet.model.step = Step::Choose;
                            sheet.model.error = Some(account_error(error));
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn credentials_next(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.internet_account_sheet.as_mut() else {
            return;
        };
        sheet.model.address = sheet.address.read(cx).value().to_string();
        sheet.model.name = sheet.name.read(cx).value().to_string();
        sheet.model.password = sheet.password.read(cx).value().to_string();
        sheet.model.caldav_uri = sheet.caldav.read(cx).value().to_string();
        if !sheet.model.credentials_valid() {
            sheet.model.error = Some("Enter your name, a valid email address, and a password.");
        } else if sheet.manual
            && (sheet.imap.read(cx).value().trim().is_empty()
                || sheet.smtp.read(cx).value().trim().is_empty())
        {
            sheet.model.error = Some("Enter both the IMAP and SMTP server names.");
        } else {
            sheet.model.error = None;
            sheet.model.step = Step::Services;
        }
        cx.notify();
    }

    fn save_account(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.internet_account_sheet.as_mut() else {
            return;
        };
        if !sheet.model.services.any() {
            sheet.model.error = Some("Choose at least one app for this account.");
            cx.notify();
            return;
        }
        let Some(choice) = sheet.model.choice else {
            return;
        };
        let services = sheet.model.services;
        let address = sheet.model.address.clone();
        let name = sheet.model.name.clone();
        let password = Secret::new(std::mem::take(&mut sheet.model.password));
        let caldav = sheet.model.caldav_uri.clone();
        let imap = sheet.imap.read(cx).value().to_string();
        let smtp = sheet.smtp.read(cx).value().to_string();
        let tokens = sheet.tokens.take();
        let manual = sheet.manual;
        let discovered_config = sheet.discovered_config.take();
        sheet.generation += 1;
        let generation = sheet.generation;
        self.internet_accounts_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                create_account(choice, services, &address, &name, &password, &caldav, &imap, &smtp, manual, discovered_config, tokens.as_ref())
            }).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(sheet) = this.internet_account_sheet.as_mut().filter(|sheet| sheet.generation == generation) {
                    this.internet_accounts_busy = false;
                    match result {
                        Ok(()) => {
                            this.internet_account_sheet = None;
                            this.refresh_internet_accounts(cx);
                        }
                        #[cfg(target_os = "linux")]
                        Err(SaveError::Manual) => {
                            sheet.manual = true;
                            sheet.model.step = Step::Credentials;
                            sheet.model.error = Some("Lulo couldn't find the mail servers. Enter the IMAP and SMTP server names.");
                        }
                        Err(SaveError::Goa(error)) => {
                            sheet.model.step = if choice.provider().info().oauth.is_some() {
                                Step::Choose
                            } else {
                                Step::Credentials
                            };
                            sheet.model.error = Some(account_error(error));
                        }
                    }
                    cx.notify();
                }
            });
        }).detach();
    }

    fn set_account_service(
        &mut self,
        path: String,
        service: Service,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.internet_accounts_busy {
            return;
        }
        self.internet_accounts_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            #[cfg(target_os = "linux")]
            let result = blocking::unblock(move || {
                rmac_accounts_linux::goa::GoaBus::session()?.set_service(&path, service, enabled)
            })
            .await;
            #[cfg(not(target_os = "linux"))]
            let result: Result<(), rmac_accounts_linux::Error> =
                Err(rmac_accounts_linux::Error::Unavailable);
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.internet_accounts_busy = false;
                match result {
                    Ok(()) => this.refresh_internet_accounts(cx),
                    Err(error) => {
                        this.internet_accounts_error = Some(account_error(error).into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn remove_account(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.internet_account_selected.clone() else {
            return;
        };
        let paths = rmac_accounts_ui::account_rows(&self.internet_accounts)
            .into_iter()
            .find(|row| row.paths.contains(&path))
            .map(|row| row.paths)
            .unwrap_or_else(|| vec![path]);
        self.internet_account_delete = false;
        self.internet_accounts_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            #[cfg(target_os = "linux")]
            let result = blocking::unblock(move || {
                let bus = rmac_accounts_linux::goa::GoaBus::session()?;
                for path in &paths {
                    bus.remove(path)?;
                }
                Ok(())
            })
            .await;
            #[cfg(not(target_os = "linux"))]
            let result: Result<(), rmac_accounts_linux::Error> =
                Err(rmac_accounts_linux::Error::Unavailable);
            #[cfg(not(target_os = "linux"))]
            let _ = paths;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.internet_accounts_busy = false;
                match result {
                    Ok(()) => {
                        this.internet_account_selected = None;
                        this.refresh_internet_accounts(cx);
                    }
                    Err(error) => {
                        this.internet_accounts_error = Some(account_error(error).into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

enum SaveError {
    Goa(rmac_accounts_linux::Error),
    #[cfg(target_os = "linux")]
    Manual,
}

#[allow(clippy::too_many_arguments)]
fn create_account(
    choice: Choice,
    services: Services,
    address: &str,
    name: &str,
    password: &Secret,
    caldav: &str,
    imap: &str,
    smtp: &str,
    manual: bool,
    discovered_config: Option<MailConfig>,
    tokens: Option<&Tokens>,
) -> Result<(), SaveError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            choice,
            services,
            address,
            name,
            password,
            caldav,
            imap,
            smtp,
            manual,
            discovered_config,
            tokens,
        );
        return Err(SaveError::Goa(rmac_accounts_linux::Error::Unavailable));
    }
    #[cfg(target_os = "linux")]
    {
        let bus = rmac_accounts_linux::goa::GoaBus::session().map_err(SaveError::Goa)?;
        if let Some(tokens) = tokens {
            bus.add_oauth(&OAuthAccount {
                provider: choice.provider(),
                identity: &tokens.identity,
                presentation_identity: &tokens.presentation_identity,
                access_token: &tokens.access_token,
                refresh_token: tokens.refresh_token.as_ref(),
                expires_at: tokens.expires_at,
                services,
            })
            .map_err(SaveError::Goa)?;
            return Ok(());
        }
        if choice == Choice::OtherCalendar {
            bus.add_password_calendar(&PasswordCalendarAccount {
                username: address,
                presentation_identity: address,
                caldav_uri: caldav,
                password,
            })
            .map_err(SaveError::Goa)?;
            return Ok(());
        }
        let config = if let Some(config) = discovered_config {
            config
        } else if manual {
            if imap.trim().is_empty() || smtp.trim().is_empty() {
                return Err(SaveError::Manual);
            }
            MailConfig {
                imap: MailServer {
                    host: imap.trim().into(),
                    port: 993,
                    security: SocketSecurity::Tls,
                    username: address.into(),
                },
                smtp: MailServer {
                    host: smtp.trim().into(),
                    port: 587,
                    security: SocketSecurity::StartTls,
                    username: address.into(),
                },
            }
        } else {
            discover(&SystemDiscovery, address)
                .map_err(SaveError::Goa)?
                .config
                .ok_or(SaveError::Manual)?
        };
        let mail = bus
            .add_password_mail(&PasswordMailAccount {
                address,
                display_name: name,
                config: &config,
                imap_password: password,
                smtp_password: password,
            })
            .map_err(SaveError::Goa)?;
        if services.calendar {
            let uri = match choice
                .provider()
                .info()
                .servers
                .and_then(|preset| preset.caldav_uri)
            {
                Some(uri) => uri,
                None if !caldav.is_empty() => caldav,
                None => return Ok(()),
            };
            if let Err(error) = bus.add_password_calendar(&PasswordCalendarAccount {
                username: address,
                presentation_identity: address,
                caldav_uri: uri,
                password,
            }) {
                let _ = bus.remove(&mail);
                return Err(SaveError::Goa(error));
            }
        }
        if !services.mail {
            bus.set_service(&mail, Service::Mail, false)
                .map_err(SaveError::Goa)?;
        }
        Ok(())
    }
}
