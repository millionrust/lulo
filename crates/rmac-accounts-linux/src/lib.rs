//! Worker-thread adapters for GNOME Online Accounts and secure mail discovery.
//! None of the blocking entry points in this crate may be called on the UI thread.

pub mod discovery;
pub mod oauth;

#[cfg(target_os = "linux")]
pub mod goa;

use rmac_accounts::{
    autoconfig::MailConfig,
    model::{Service, Services},
    provider::Provider,
    Secret,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Busy,
    InvalidResponse,
    Network,
    SignInFailed,
}

/// External identity data is deliberately absent from diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct GoaAccount {
    pub path: String,
    pub id: String,
    pub provider: String,
    pub identity: String,
    pub services: Services,
}

impl std::fmt::Debug for GoaAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoaAccount")
            .field("path", &"[redacted]")
            .field("id", &"[redacted]")
            .field("provider", &self.provider)
            .field("identity", &"[redacted]")
            .field("services", &self.services)
            .finish()
    }
}

pub enum AccountChange {
    Added(GoaAccount),
    Removed(String),
    Updated(GoaAccount),
}

/// GOA is the sole credential store. This boundary is mockable in tests.
pub trait GoaApi {
    fn accounts(&self) -> Result<Vec<GoaAccount>, Error>;
    /// Blocks on ObjectManager and PropertiesChanged signals. Run on a worker.
    fn watch(&self, emit: &mut dyn FnMut(AccountChange)) -> Result<(), Error>;
    fn add_oauth(&self, account: &OAuthAccount<'_>) -> Result<String, Error>;
    fn add_password_mail(&self, account: &PasswordMailAccount<'_>) -> Result<String, Error>;
    fn add_password_calendar(&self, account: &PasswordCalendarAccount<'_>)
        -> Result<String, Error>;
    fn remove(&self, path: &str) -> Result<(), Error>;
    fn set_service(&self, path: &str, service: Service, enabled: bool) -> Result<(), Error>;
    fn access_token(&self, path: &str) -> Result<Secret, Error>;
    /// `id` is `imap-password`, `smtp-password`, or a provider-specific key.
    fn password(&self, path: &str, id: &str) -> Result<Secret, Error>;
}

pub struct OAuthAccount<'a> {
    pub provider: Provider,
    pub identity: &'a str,
    pub presentation_identity: &'a str,
    pub access_token: &'a Secret,
    pub refresh_token: Option<&'a Secret>,
    pub expires_at: i64,
    pub services: Services,
}

impl std::fmt::Debug for OAuthAccount<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuthAccount([redacted])")
    }
}

pub struct PasswordMailAccount<'a> {
    pub address: &'a str,
    pub display_name: &'a str,
    pub config: &'a MailConfig,
    pub imap_password: &'a Secret,
    pub smtp_password: &'a Secret,
}

impl std::fmt::Debug for PasswordMailAccount<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordMailAccount([redacted])")
    }
}

pub struct PasswordCalendarAccount<'a> {
    pub username: &'a str,
    pub presentation_identity: &'a str,
    pub caldav_uri: &'a str,
    pub password: &'a Secret,
}

impl std::fmt::Debug for PasswordCalendarAccount<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordCalendarAccount([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeGoa {
        account: Mutex<Option<GoaAccount>>,
    }

    impl GoaApi for FakeGoa {
        fn accounts(&self) -> Result<Vec<GoaAccount>, Error> {
            Ok(self.account.lock().unwrap().iter().cloned().collect())
        }
        fn watch(&self, emit: &mut dyn FnMut(AccountChange)) -> Result<(), Error> {
            for account in self.accounts()? {
                emit(AccountChange::Added(account));
            }
            Ok(())
        }
        fn add_oauth(&self, input: &OAuthAccount<'_>) -> Result<String, Error> {
            let path = "/org/gnome/OnlineAccounts/Accounts/fake".to_owned();
            *self.account.lock().unwrap() = Some(GoaAccount {
                path: path.clone(),
                id: "fake".into(),
                provider: input.provider.info().goa_ids[0].into(),
                identity: input.identity.into(),
                services: input.services,
            });
            Ok(path)
        }
        fn add_password_mail(&self, _account: &PasswordMailAccount<'_>) -> Result<String, Error> {
            Ok("/org/gnome/OnlineAccounts/Accounts/password-fake".into())
        }
        fn add_password_calendar(
            &self,
            _account: &PasswordCalendarAccount<'_>,
        ) -> Result<String, Error> {
            Ok("/org/gnome/OnlineAccounts/Accounts/calendar-fake".into())
        }
        fn remove(&self, _path: &str) -> Result<(), Error> {
            *self.account.lock().unwrap() = None;
            Ok(())
        }
        fn set_service(&self, _path: &str, service: Service, enabled: bool) -> Result<(), Error> {
            self.account
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .services
                .set(service, enabled);
            Ok(())
        }
        fn access_token(&self, _path: &str) -> Result<Secret, Error> {
            Ok(Secret::new("planted-token".into()))
        }
        fn password(&self, _path: &str, _id: &str) -> Result<Secret, Error> {
            Ok(Secret::new("planted-password".into()))
        }
    }

    #[test]
    fn account_lifecycle_with_fake_goa_and_redacted_diagnostics() {
        let fake = FakeGoa::default();
        let access = Secret::new("planted-token".into());
        let refresh = Secret::new("planted-refresh".into());
        let input = OAuthAccount {
            provider: Provider::Google,
            identity: "planted@example.com",
            presentation_identity: "planted@example.com",
            access_token: &access,
            refresh_token: Some(&refresh),
            expires_at: 123,
            services: Services::ALL,
        };
        let path = fake.add_oauth(&input).unwrap();
        let mut events = Vec::new();
        fake.watch(&mut |event| events.push(event)).unwrap();
        assert_eq!(events.len(), 1);
        fake.set_service(&path, Service::Calendar, false).unwrap();
        assert!(!fake.accounts().unwrap()[0].services.calendar);
        assert_eq!(fake.access_token(&path).unwrap().expose(), "planted-token");
        assert_eq!(
            fake.password(&path, "imap-password").unwrap().expose(),
            "planted-password"
        );
        let diagnostic = format!(
            "{input:?} {:?} {:?}",
            fake.accounts().unwrap()[0],
            fake.access_token(&path).unwrap()
        );
        for secret in ["planted-token", "planted@example.com", "planted-password"] {
            assert!(!diagnostic.contains(secret));
        }
        fake.remove(&path).unwrap();
        assert!(fake.accounts().unwrap().is_empty());
    }
}
