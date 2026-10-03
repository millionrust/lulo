//! Worker-thread adapters for GNOME Online Accounts and secure mail discovery.
//! None of the blocking entry points in this crate may be called on the UI thread.

pub mod discovery;
pub mod oauth;

#[cfg(target_os = "linux")]
pub mod goa;

use rmac_accounts::{
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
    fn remove(&self, path: &str) -> Result<(), Error>;
    fn set_service(&self, path: &str, service: Service, enabled: bool) -> Result<(), Error>;
    fn access_token(&self, path: &str) -> Result<Secret, Error>;
    fn password(&self, path: &str) -> Result<Secret, Error>;
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
