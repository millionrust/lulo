//! GOA D-Bus calls. The session connection and signal iterator block; callers
//! must own a worker thread and never pass secrets to a logger.

use std::collections::HashMap;

use rmac_accounts::Secret;
use rmac_accounts::{
    model::{email_domain, Service, Services},
    provider::SocketSecurity,
};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};

use crate::{
    AccountChange, Error, GoaAccount, GoaApi, OAuthAccount, PasswordCalendarAccount,
    PasswordMailAccount,
};

const SERVICE: &str = "org.gnome.OnlineAccounts";
const ROOT: &str = "/org/gnome/OnlineAccounts";
const ACCOUNT_INTERFACE: &str = "org.gnome.OnlineAccounts.Account";

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;

pub struct GoaBus {
    connection: Connection,
}

impl GoaBus {
    pub fn session() -> Result<Self, Error> {
        Ok(Self {
            connection: Connection::session().map_err(|_| Error::Unavailable)?,
        })
    }

    pub fn on_connection(connection: Connection) -> Self {
        Self { connection }
    }

    fn proxy<'a>(&'a self, path: &'a str, interface: &'a str) -> Result<Proxy<'a>, Error> {
        Proxy::new(&self.connection, SERVICE, path, interface).map_err(|_| Error::Unavailable)
    }

    fn account<'a>(&'a self, path: &'a str) -> Result<Proxy<'a>, Error> {
        self.proxy(path, ACCOUNT_INTERFACE)
    }
}

fn property_string(values: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    String::try_from(values.get(key)?.try_clone().ok()?).ok()
}

fn account_from_interfaces(path: &str, interfaces: &Interfaces) -> Option<GoaAccount> {
    let props = interfaces.get(ACCOUNT_INTERFACE)?;
    Some(GoaAccount {
        path: path.to_owned(),
        id: property_string(props, "Id")?,
        provider: property_string(props, "ProviderType")?,
        identity: property_string(props, "PresentationIdentity")?,
        services: Services {
            mail: interfaces.contains_key("org.gnome.OnlineAccounts.Mail"),
            calendar: interfaces.contains_key("org.gnome.OnlineAccounts.Calendar"),
            contacts: interfaces.contains_key("org.gnome.OnlineAccounts.Contacts"),
        },
    })
}

fn string_value(value: &str) -> OwnedValue {
    OwnedValue::from(Str::from(value.to_owned()))
}

fn server_address(
    host: &str,
    port: u16,
    security: SocketSecurity,
    imap: bool,
) -> Result<String, Error> {
    let url = url::Url::parse(&format!("https://{host}")).map_err(|_| Error::InvalidResponse)?;
    if url.host_str() != Some(host) || url.port().is_some() || url.path() != "/" || port == 0 {
        return Err(Error::InvalidResponse);
    }
    let default = match (imap, security) {
        (true, SocketSecurity::Tls) => 993,
        (true, SocketSecurity::StartTls) => 143,
        (false, SocketSecurity::Tls) => 465,
        (false, SocketSecurity::StartTls) => 587,
    };
    Ok(if port == default {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    })
}

impl GoaApi for GoaBus {
    fn accounts(&self) -> Result<Vec<GoaAccount>, Error> {
        let manager = self.proxy(ROOT, "org.freedesktop.DBus.ObjectManager")?;
        let objects: Objects = manager
            .call("GetManagedObjects", &())
            .map_err(|_| Error::Unavailable)?;
        let mut accounts: Vec<_> = objects
            .iter()
            .filter_map(|(path, interfaces)| account_from_interfaces(path.as_str(), interfaces))
            .collect();
        accounts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(accounts)
    }

    fn watch(&self, emit: &mut dyn FnMut(AccountChange)) -> Result<(), Error> {
        // Subscribing before the initial snapshot closes the startup race.
        let rule = "type='signal',sender='org.gnome.OnlineAccounts'";
        let iterator = MessageIterator::for_match_rule(rule, &self.connection, Some(32))
            .map_err(|_| Error::Unavailable)?;
        let mut previous: HashMap<_, _> = self
            .accounts()?
            .into_iter()
            .map(|account| (account.path.clone(), account))
            .collect();
        for message in iterator {
            let message = message.map_err(|_| Error::Unavailable)?;
            let header = message.header();
            let interface = header.interface().map(|name| name.as_str());
            if !matches!(
                interface,
                Some("org.freedesktop.DBus.ObjectManager" | "org.freedesktop.DBus.Properties")
            ) {
                continue;
            }
            let current: HashMap<_, _> = self
                .accounts()?
                .into_iter()
                .map(|account| (account.path.clone(), account))
                .collect();
            for (path, account) in &current {
                match previous.get(path) {
                    None => emit(AccountChange::Added(account.clone())),
                    Some(old) if old != account => emit(AccountChange::Updated(account.clone())),
                    _ => {}
                }
            }
            for path in previous.keys() {
                if !current.contains_key(path) {
                    emit(AccountChange::Removed(path.clone()));
                }
            }
            previous = current;
        }
        Err(Error::Unavailable)
    }

    fn add_oauth(&self, account: &OAuthAccount<'_>) -> Result<String, Error> {
        let provider = account.provider.info();
        if provider.oauth.is_none()
            || account.identity.is_empty()
            || account.access_token.expose().is_empty()
            || account.refresh_token.is_none()
            || account.expires_at <= 0
        {
            return Err(Error::InvalidResponse);
        }
        let manager = self.proxy(
            "/org/gnome/OnlineAccounts/Manager",
            "org.gnome.OnlineAccounts.Manager",
        )?;
        let mut credentials = HashMap::from([
            (
                "access_token".to_owned(),
                string_value(account.access_token.expose()),
            ),
            (
                "access_token_expires_at".to_owned(),
                OwnedValue::from(account.expires_at),
            ),
        ]);
        if let Some(refresh_token) = account.refresh_token {
            credentials.insert("refresh_token".into(), string_value(refresh_token.expose()));
        }
        let bool_string = |value: bool| if value { "true" } else { "false" }.to_owned();
        let details = HashMap::from([
            ("MailEnabled".to_owned(), bool_string(account.services.mail)),
            (
                "CalendarEnabled".to_owned(),
                bool_string(account.services.calendar),
            ),
            (
                "ContactsEnabled".to_owned(),
                bool_string(account.services.contacts),
            ),
            ("FilesEnabled".to_owned(), "false".to_owned()),
        ]);
        let mut details = details;
        if account.provider == rmac_accounts::provider::Provider::Microsoft {
            let oauth = provider.oauth.ok_or(Error::InvalidResponse)?;
            details.extend([
                (
                    "OAuth2AuthorizationUri".into(),
                    oauth.authorization_uri.into(),
                ),
                ("OAuth2TokenUri".into(), oauth.token_uri.into()),
                ("OAuth2ClientId".into(), oauth.client_id.into()),
                ("OAuth2RedirectUri".into(), oauth.redirect_uri.into()),
                ("OAuth2ClientSecret".into(), oauth.client_secret.into()),
            ]);
        }
        let path: OwnedObjectPath = manager
            .call(
                "AddAccount",
                &(
                    provider.goa_ids[0],
                    account.identity,
                    account.presentation_identity,
                    credentials,
                    details,
                ),
            )
            .map_err(|_| Error::SignInFailed)?;
        Ok(path.to_string())
    }

    fn add_password_mail(&self, account: &PasswordMailAccount<'_>) -> Result<String, Error> {
        if email_domain(account.address).is_none()
            || account.display_name.trim().is_empty()
            || account.imap_password.expose().is_empty()
            || account.smtp_password.expose().is_empty()
            || account.config.imap.username.is_empty()
            || account.config.smtp.username.is_empty()
        {
            return Err(Error::InvalidResponse);
        }
        let imap = &account.config.imap;
        let smtp = &account.config.smtp;
        let imap_host = server_address(&imap.host, imap.port, imap.security, true)?;
        let smtp_host = server_address(&smtp.host, smtp.port, smtp.security, false)?;
        let credentials = HashMap::from([
            (
                "imap-password".to_owned(),
                string_value(account.imap_password.expose()),
            ),
            (
                "smtp-password".to_owned(),
                string_value(account.smtp_password.expose()),
            ),
        ]);
        let mut details = HashMap::from([
            ("Enabled".to_owned(), "true".to_owned()),
            ("EmailAddress".to_owned(), account.address.to_owned()),
            ("Name".to_owned(), account.display_name.to_owned()),
            ("ImapHost".to_owned(), imap_host),
            ("ImapUserName".to_owned(), imap.username.clone()),
            ("SmtpHost".to_owned(), smtp_host),
            ("SmtpUserName".to_owned(), smtp.username.clone()),
            ("SmtpUseAuth".to_owned(), "true".to_owned()),
            ("SmtpAuthPlain".to_owned(), "true".to_owned()),
            ("SmtpAuthLogin".to_owned(), "true".to_owned()),
            ("ImapAcceptSslErrors".to_owned(), "false".to_owned()),
            ("SmtpAcceptSslErrors".to_owned(), "false".to_owned()),
        ]);
        let enabled = |value: bool| if value { "true" } else { "false" }.to_owned();
        details.insert(
            "ImapUseSsl".into(),
            enabled(imap.security == SocketSecurity::Tls),
        );
        details.insert(
            "ImapUseTls".into(),
            enabled(imap.security == SocketSecurity::StartTls),
        );
        details.insert(
            "SmtpUseSsl".into(),
            enabled(smtp.security == SocketSecurity::Tls),
        );
        details.insert(
            "SmtpUseTls".into(),
            enabled(smtp.security == SocketSecurity::StartTls),
        );
        let manager = self.proxy(
            "/org/gnome/OnlineAccounts/Manager",
            "org.gnome.OnlineAccounts.Manager",
        )?;
        let path: OwnedObjectPath = manager
            .call(
                "AddAccount",
                &(
                    "imap_smtp",
                    account.address,
                    account.address,
                    credentials,
                    details,
                ),
            )
            .map_err(|_| Error::SignInFailed)?;
        Ok(path.to_string())
    }

    fn add_password_calendar(
        &self,
        account: &PasswordCalendarAccount<'_>,
    ) -> Result<String, Error> {
        let uri = url::Url::parse(account.caldav_uri).map_err(|_| Error::InvalidResponse)?;
        if uri.scheme() != "https"
            || uri.host_str().is_none()
            || uri.username() != ""
            || uri.password().is_some()
            || uri.fragment().is_some()
            || uri.query().is_some()
            || account.username.is_empty()
            || account.presentation_identity.is_empty()
            || account.password.expose().is_empty()
        {
            return Err(Error::InvalidResponse);
        }
        let credentials = HashMap::from([(
            "password".to_owned(),
            string_value(account.password.expose()),
        )]);
        let details = HashMap::from([
            ("Uri".to_owned(), "".to_owned()),
            ("CalendarEnabled".to_owned(), "true".to_owned()),
            ("CalDavUri".to_owned(), account.caldav_uri.to_owned()),
            ("ContactsEnabled".to_owned(), "false".to_owned()),
            ("CardDavUri".to_owned(), "".to_owned()),
            ("FilesEnabled".to_owned(), "false".to_owned()),
            ("AcceptSslErrors".to_owned(), "false".to_owned()),
        ]);
        let manager = self.proxy(
            "/org/gnome/OnlineAccounts/Manager",
            "org.gnome.OnlineAccounts.Manager",
        )?;
        let path: OwnedObjectPath = manager
            .call(
                "AddAccount",
                &(
                    "webdav",
                    account.username,
                    account.presentation_identity,
                    credentials,
                    details,
                ),
            )
            .map_err(|_| Error::SignInFailed)?;
        Ok(path.to_string())
    }

    fn remove(&self, path: &str) -> Result<(), Error> {
        self.account(path)?
            .call("Remove", &())
            .map_err(|_| Error::Unavailable)
    }

    fn set_service(&self, path: &str, service: Service, enabled: bool) -> Result<(), Error> {
        let property = match service {
            Service::Mail => "MailDisabled",
            Service::Calendar => "CalendarDisabled",
            Service::Contacts => "ContactsDisabled",
        };
        self.account(path)?
            .set_property(property, !enabled)
            .map_err(|_| Error::Unavailable)
    }

    fn access_token(&self, path: &str) -> Result<Secret, Error> {
        self.account(path)?
            .call::<_, _, i32>("EnsureCredentials", &())
            .map_err(|_| Error::SignInFailed)?;
        let (token, _expires): (String, i32) = self
            .proxy(path, "org.gnome.OnlineAccounts.OAuth2Based")?
            .call("GetAccessToken", &())
            .map_err(|_| Error::SignInFailed)?;
        Ok(Secret::new(token))
    }

    fn password(&self, path: &str, id: &str) -> Result<Secret, Error> {
        let password: String = self
            .proxy(path, "org.gnome.OnlineAccounts.PasswordBased")?
            .call("GetPassword", &(id,))
            .map_err(|_| Error::SignInFailed)?;
        Ok(Secret::new(password))
    }
}
