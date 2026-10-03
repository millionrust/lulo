//! GOA D-Bus calls. The session connection and signal iterator block; callers
//! must own a worker thread and never pass secrets to a logger.

use std::collections::HashMap;

use rmac_accounts::model::{Service, Services};
use rmac_accounts::Secret;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};

use crate::{AccountChange, Error, GoaAccount, GoaApi, OAuthAccount};

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
        if provider.oauth.is_none() || account.identity.is_empty() {
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
        ]);
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

    fn password(&self, path: &str) -> Result<Secret, Error> {
        let password: String = self
            .proxy(path, "org.gnome.OnlineAccounts.PasswordBased")?
            .call("GetPassword", &())
            .map_err(|_| Error::SignInFailed)?;
        Ok(Secret::new(password))
    }
}
