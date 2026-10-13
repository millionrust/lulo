//! Lulo's own account store on Windows, standing in for GNOME Online
//! Accounts (ADR 0023 phase 2f).
//!
//! Account details that are not secret (address, provider, which services,
//! mail servers) live in `accounts.json` under Lulo's config folder; every
//! password and token lives in the Windows Credential Manager
//! (`crate::wincred`), under the account's id. `WinAccounts` implements the
//! same [`GoaApi`] the Linux GOA adapter does, so Mail and System Settings
//! reach accounts through one interface on both systems.
//!
//! Blocking, like the GOA adapter: call it from a worker, never the UI thread.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_accounts::autoconfig::{MailConfig, MailServer};
use rmac_accounts::model::{Service, Services};
use rmac_accounts::provider::{Provider, SocketSecurity};
use rmac_accounts::Secret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    wincred, AccountChange, Error, GoaAccount, GoaApi, OAuthAccount, PasswordCalendarAccount,
    PasswordMailAccount,
};

const FILE: &str = "lulo/accounts.json";
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_ACCOUNTS: usize = 64;
const PREFIX: &str = "win:";

/// How an account signs in.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An access token refreshed with the stored refresh token.
    OAuth,
    /// A password (an app password for iCloud and Yahoo).
    Password,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredServer {
    pub host: String,
    pub port: u16,
    /// `tls` (implicit) or `starttls`.
    pub security: String,
    pub username: String,
}

impl StoredServer {
    fn from_server(server: &MailServer) -> Self {
        Self {
            host: server.host.clone(),
            port: server.port,
            security: match server.security {
                SocketSecurity::Tls => "tls",
                SocketSecurity::StartTls => "starttls",
            }
            .into(),
            username: server.username.clone(),
        }
    }

    pub fn security(&self) -> SocketSecurity {
        if self.security == "starttls" {
            SocketSecurity::StartTls
        } else {
            SocketSecurity::Tls
        }
    }
}

/// One account as the index file records it. No field here is a secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredAccount {
    pub id: String,
    /// GOA's provider names, which the rest of Lulo already switches on:
    /// `google`, `ms_graph` or `imap_smtp`.
    pub provider: String,
    pub kind: Kind,
    pub identity: String,
    pub display_name: String,
    pub mail: bool,
    pub calendar: bool,
    pub contacts: bool,
    pub imap: Option<StoredServer>,
    pub smtp: Option<StoredServer>,
    pub caldav_uri: Option<String>,
    /// OAuth access token expiry, microseconds since the Unix epoch.
    pub expires_at: i64,
}

impl StoredAccount {
    pub fn path(&self) -> String {
        format!("{PREFIX}{}", self.id)
    }

    pub fn services(&self) -> Services {
        Services {
            mail: self.mail,
            calendar: self.calendar,
            contacts: self.contacts,
        }
    }

    pub fn to_goa(&self) -> GoaAccount {
        GoaAccount {
            path: self.path(),
            id: self.id.clone(),
            provider: self.provider.clone(),
            identity: self.identity.clone(),
            services: self.services(),
        }
    }

    pub fn mail_config(&self) -> Option<MailConfig> {
        let (imap, smtp) = (self.imap.as_ref()?, self.smtp.as_ref()?);
        let server = |stored: &StoredServer| MailServer {
            host: stored.host.clone(),
            port: stored.port,
            security: stored.security(),
            username: stored.username.clone(),
        };
        Some(MailConfig {
            imap: server(imap),
            smtp: server(smtp),
        })
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct Index {
    accounts: Vec<StoredAccount>,
}

fn secret_name(id: &str, kind: &str) -> String {
    format!("Lulo/account/{id}/{kind}")
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("Lulo/Config"))
        })
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_micros() as i64)
        .unwrap_or(0)
}

/// A fresh opaque account id (not a secret): 128 bits from the OS.
fn new_id() -> Result<String, Error> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Unavailable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The account store. Cheap to construct; every call re-reads the index, so
/// two Lulo apps never act on a stale list.
pub struct WinAccounts {
    index: PathBuf,
}

impl WinAccounts {
    pub fn session() -> Result<Self, Error> {
        let root = config_root().ok_or(Error::Unavailable)?;
        Ok(Self::at(root.join(FILE)))
    }

    /// An account store whose index file is `index` (tests, and a launch
    /// check that points an app at a private profile).
    pub fn at(index: PathBuf) -> Self {
        Self { index }
    }

    fn load(&self) -> Result<Index, Error> {
        match rmac_storage::read_bounded_no_follow(&self.index, MAX_FILE_BYTES) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(_) => Err(Error::Unavailable),
        }
    }

    fn save(&self, index: &Index) -> Result<(), Error> {
        let bytes = serde_json::to_vec_pretty(index).map_err(|_| Error::InvalidResponse)?;
        let parent: &Path = self.index.parent().ok_or(Error::Unavailable)?;
        rmac_storage::create_dir_all_private(parent).map_err(|_| Error::Unavailable)?;
        rmac_storage::atomic_write_private(&self.index, &bytes).map_err(|_| Error::Unavailable)
    }

    /// Every account with its non-secret details.
    pub fn stored(&self) -> Result<Vec<StoredAccount>, Error> {
        Ok(self.load()?.accounts)
    }

    pub fn stored_by_path(&self, path: &str) -> Result<StoredAccount, Error> {
        let id = path.strip_prefix(PREFIX).ok_or(Error::InvalidResponse)?;
        self.load()?
            .accounts
            .into_iter()
            .find(|account| account.id == id)
            .ok_or(Error::InvalidResponse)
    }

    /// The id an account will have: the existing one for an address already
    /// added with this provider, otherwise a fresh one. Chosen before the
    /// secrets are saved so they land under the id the index keeps.
    fn id_for(&self, provider: &str, identity: &str) -> Result<String, Error> {
        let existing = self.load()?.accounts.into_iter().find(|existing| {
            existing.provider == provider && existing.identity.eq_ignore_ascii_case(identity)
        });
        match existing {
            Some(existing) => Ok(existing.id),
            None => new_id(),
        }
    }

    fn push(&self, mut account: StoredAccount) -> Result<String, Error> {
        let mut index = self.load()?;
        if index.accounts.len() >= MAX_ACCOUNTS {
            return Err(Error::Busy);
        }
        // The same address added twice replaces the first, as Internet
        // Accounts does not allow two of one identity for one provider.
        if let Some(existing) = index.accounts.iter().position(|existing| {
            existing.provider == account.provider
                && existing.identity.eq_ignore_ascii_case(&account.identity)
        }) {
            account.id = index.accounts[existing].id.clone();
            index.accounts[existing] = account.clone();
        } else {
            index.accounts.push(account.clone());
        }
        self.save(&index)?;
        Ok(account.path())
    }

    fn save_secret(&self, id: &str, kind: &str, secret: &Secret) -> Result<(), Error> {
        wincred::write_secret(&secret_name(id, kind), secret.expose())
    }

    fn read_secret(&self, id: &str, kind: &str) -> Result<Secret, Error> {
        wincred::read_secret(&secret_name(id, kind))?
            .map(Secret::new)
            .ok_or(Error::Unavailable)
    }

    fn forget_secrets(id: &str) {
        for kind in [
            "imap-password",
            "smtp-password",
            "caldav-password",
            "access-token",
            "refresh-token",
        ] {
            wincred::delete_secret(&secret_name(id, kind));
        }
    }

    /// The access token, refreshed first when it expires within a minute.
    fn fresh_access_token(&self, account: &StoredAccount) -> Result<Secret, Error> {
        let provider = match account.provider.as_str() {
            "google" => Provider::Google,
            "ms_graph" => Provider::Microsoft,
            _ => return Err(Error::InvalidResponse),
        };
        if account.expires_at > now_micros().saturating_add(60_000_000) {
            return self.read_secret(&account.id, "access-token");
        }
        let refresh = self.read_secret(&account.id, "refresh-token")?;
        let refreshed = crate::oauth::refresh_access_token(provider, &refresh)?;
        self.save_secret(&account.id, "access-token", &refreshed.access_token)?;
        if let Some(rotated) = &refreshed.refresh_token {
            self.save_secret(&account.id, "refresh-token", rotated)?;
        }
        let mut index = self.load()?;
        if let Some(stored) = index
            .accounts
            .iter_mut()
            .find(|stored| stored.id == account.id)
        {
            stored.expires_at = refreshed.expires_at;
        }
        self.save(&index)?;
        Ok(refreshed.access_token)
    }
}

impl GoaApi for WinAccounts {
    fn accounts(&self) -> Result<Vec<GoaAccount>, Error> {
        Ok(self
            .load()?
            .accounts
            .iter()
            .map(StoredAccount::to_goa)
            .collect())
    }

    /// Windows has no account-change signal, and Lulo's apps add and remove
    /// accounts themselves, so this reports the accounts there are and
    /// returns instead of blocking.
    fn watch(&self, emit: &mut dyn FnMut(AccountChange)) -> Result<(), Error> {
        for account in self.accounts()? {
            emit(AccountChange::Added(account));
        }
        Ok(())
    }

    fn add_oauth(&self, account: &OAuthAccount<'_>) -> Result<String, Error> {
        let info = account.provider.info();
        if info.oauth.is_none() || account.refresh_token.is_none() {
            return Err(Error::InvalidResponse);
        }
        let id = self.id_for(info.goa_ids[0], account.identity)?;
        self.save_secret(&id, "access-token", account.access_token)?;
        if let Some(refresh) = account.refresh_token {
            self.save_secret(&id, "refresh-token", refresh)?;
        }
        let stored = StoredAccount {
            id: id.clone(),
            provider: info.goa_ids[0].into(),
            kind: Kind::OAuth,
            identity: account.identity.into(),
            display_name: account.presentation_identity.into(),
            mail: account.services.mail,
            calendar: account.services.calendar,
            contacts: account.services.contacts,
            imap: None,
            smtp: None,
            caldav_uri: None,
            expires_at: account.expires_at,
        };
        self.push(stored)
    }

    fn add_password_mail(&self, account: &PasswordMailAccount<'_>) -> Result<String, Error> {
        let id = self.id_for("imap_smtp", account.address)?;
        self.save_secret(&id, "imap-password", account.imap_password)?;
        self.save_secret(&id, "smtp-password", account.smtp_password)?;
        let stored = StoredAccount {
            id: id.clone(),
            provider: "imap_smtp".into(),
            kind: Kind::Password,
            identity: account.address.into(),
            display_name: account.display_name.into(),
            mail: true,
            calendar: false,
            contacts: false,
            imap: Some(StoredServer::from_server(&account.config.imap)),
            smtp: Some(StoredServer::from_server(&account.config.smtp)),
            caldav_uri: None,
            expires_at: 0,
        };
        self.push(stored)
    }

    fn add_password_calendar(
        &self,
        account: &PasswordCalendarAccount<'_>,
    ) -> Result<String, Error> {
        let id = self.id_for("caldav", account.username)?;
        self.save_secret(&id, "caldav-password", account.password)?;
        let stored = StoredAccount {
            id: id.clone(),
            provider: "caldav".into(),
            kind: Kind::Password,
            identity: account.username.into(),
            display_name: account.presentation_identity.into(),
            mail: false,
            calendar: true,
            contacts: false,
            imap: None,
            smtp: None,
            caldav_uri: Some(account.caldav_uri.into()),
            expires_at: 0,
        };
        self.push(stored)
    }

    fn remove(&self, path: &str) -> Result<(), Error> {
        let id = path.strip_prefix(PREFIX).ok_or(Error::InvalidResponse)?;
        let mut index = self.load()?;
        let before = index.accounts.len();
        index.accounts.retain(|account| account.id != id);
        if index.accounts.len() == before {
            return Err(Error::InvalidResponse);
        }
        self.save(&index)?;
        Self::forget_secrets(id);
        Ok(())
    }

    fn set_service(&self, path: &str, service: Service, enabled: bool) -> Result<(), Error> {
        let id = path.strip_prefix(PREFIX).ok_or(Error::InvalidResponse)?;
        let mut index = self.load()?;
        let account = index
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .ok_or(Error::InvalidResponse)?;
        match service {
            Service::Mail => account.mail = enabled,
            Service::Calendar => account.calendar = enabled,
            Service::Contacts => account.contacts = enabled,
        }
        self.save(&index)
    }

    fn access_token(&self, path: &str) -> Result<Secret, Error> {
        let account = self.stored_by_path(path)?;
        if account.kind != Kind::OAuth {
            return Err(Error::InvalidResponse);
        }
        self.fresh_access_token(&account)
    }

    fn password(&self, path: &str, id: &str) -> Result<Secret, Error> {
        let account = self.stored_by_path(path)?;
        if account.kind != Kind::Password
            || !matches!(id, "imap-password" | "smtp-password" | "caldav-password")
        {
            return Err(Error::InvalidResponse);
        }
        self.read_secret(&account.id, id)
    }
}

/// A stable id for a stored account's cache folder, the same derivation
/// Mail uses for a GOA account (`account_id` in `rmac-mail-runtime`).
pub fn cache_key(account: &StoredAccount) -> [u8; 16] {
    let hash = Sha256::digest(format!("{}:{}", account.provider, account.id).as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(label: &str) -> (WinAccounts, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("lulo-accounts-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (WinAccounts::at(dir.join("lulo/accounts.json")), dir)
    }

    fn config() -> MailConfig {
        let server = |host: &str, port, security| MailServer {
            host: host.into(),
            port,
            security,
            username: "ada@icloud.com".into(),
        };
        MailConfig {
            imap: server("imap.mail.me.com", 993, SocketSecurity::Tls),
            smtp: server("smtp.mail.me.com", 587, SocketSecurity::StartTls),
        }
    }

    #[test]
    fn a_password_account_keeps_its_secrets_out_of_the_index() {
        let (accounts, dir) = store("password");
        let imap = Secret::new("planted-imap-password".into());
        let smtp = Secret::new("planted-smtp-password".into());
        let path = accounts
            .add_password_mail(&PasswordMailAccount {
                address: "ada@icloud.com",
                display_name: "Ada",
                config: &config(),
                imap_password: &imap,
                smtp_password: &smtp,
            })
            .unwrap();
        let listed = accounts.accounts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, path);
        assert_eq!(listed[0].provider, "imap_smtp");
        let file = std::fs::read_to_string(dir.join("lulo/accounts.json")).unwrap();
        assert!(file.contains("imap.mail.me.com"));
        assert!(!file.contains("planted-imap-password"));
        assert!(!file.contains("planted-smtp-password"));
        assert_eq!(
            accounts.password(&path, "imap-password").unwrap().expose(),
            "planted-imap-password"
        );
        assert_eq!(
            accounts.password(&path, "smtp-password").unwrap().expose(),
            "planted-smtp-password"
        );
        assert!(accounts.access_token(&path).is_err());
        let stored = accounts.stored_by_path(&path).unwrap();
        assert_eq!(stored.mail_config().unwrap(), config());
        accounts.remove(&path).unwrap();
        assert!(accounts.accounts().unwrap().is_empty());
        assert!(accounts.password(&path, "imap-password").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn adding_the_same_address_twice_keeps_one_account() {
        let (accounts, dir) = store("twice");
        let first = Secret::new("one".into());
        let second = Secret::new("two".into());
        for password in [&first, &second] {
            accounts
                .add_password_mail(&PasswordMailAccount {
                    address: "ada@icloud.com",
                    display_name: "Ada",
                    config: &config(),
                    imap_password: password,
                    smtp_password: password,
                })
                .unwrap();
        }
        let listed = accounts.accounts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            accounts
                .password(&listed[0].path, "imap-password")
                .unwrap()
                .expose(),
            "two"
        );
        accounts.remove(&listed[0].path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn services_can_be_turned_off_and_unknown_paths_are_refused() {
        let (accounts, dir) = store("services");
        let password = Secret::new("p".into());
        let path = accounts
            .add_password_mail(&PasswordMailAccount {
                address: "ada@icloud.com",
                display_name: "Ada",
                config: &config(),
                imap_password: &password,
                smtp_password: &password,
            })
            .unwrap();
        accounts.set_service(&path, Service::Mail, false).unwrap();
        assert!(!accounts.accounts().unwrap()[0].services.mail);
        assert!(accounts.remove("win:nope").is_err());
        assert!(accounts
            .remove("/org/gnome/OnlineAccounts/Accounts/1")
            .is_err());
        accounts.remove(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_oauth_account_stores_both_tokens_and_reads_a_fresh_one_back() {
        let (accounts, dir) = store("oauth");
        let access = Secret::new("a".repeat(3_000));
        let refresh = Secret::new("planted-refresh".into());
        let path = accounts
            .add_oauth(&OAuthAccount {
                provider: Provider::Microsoft,
                identity: "ada@outlook.com",
                presentation_identity: "ada@outlook.com",
                access_token: &access,
                refresh_token: Some(&refresh),
                expires_at: now_micros() + 3_600_000_000,
                services: Services::ALL,
            })
            .unwrap();
        assert_eq!(accounts.accounts().unwrap()[0].provider, "ms_graph");
        assert_eq!(accounts.access_token(&path).unwrap().expose().len(), 3_000);
        assert!(accounts.password(&path, "imap-password").is_err());
        accounts.remove(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
