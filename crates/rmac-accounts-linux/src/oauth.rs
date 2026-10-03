//! OAuth authorization-code flow with PKCE S256. Network work and waiting for
//! the browser callback are worker-thread operations.

use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rmac_accounts::{
    provider::{OAuthConfig, Provider},
    sign_in::SignIn,
    Secret,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::Error;

pub struct OAuthAttempt {
    provider: Provider,
    verifier: Secret,
    flow: SignIn,
    authorization_url: Url,
}

impl std::fmt::Debug for OAuthAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuthAttempt([redacted])")
    }
}

pub struct Tokens {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    /// GOA's absolute expiry format: microseconds since Unix epoch.
    pub expires_at: i64,
    pub identity: String,
    pub presentation_identity: String,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens([redacted])")
    }
}

pub trait OAuthHttp {
    fn exchange(
        &self,
        config: OAuthConfig,
        code: &Secret,
        verifier: &Secret,
    ) -> Result<Value, Error>;
    fn identity(&self, config: OAuthConfig, access_token: &Secret) -> Result<Value, Error>;
}

pub struct SystemOAuthHttp;

#[cfg(target_os = "linux")]
pub fn verify_installed_provider(provider: Provider) -> Result<(), Error> {
    let architecture = std::env::consts::ARCH;
    let triplet = match architecture {
        "x86_64" => "x86_64-linux-gnu",
        "aarch64" => "aarch64-linux-gnu",
        _ => return Err(Error::Unavailable),
    };
    let path = format!("/usr/lib/{triplet}/libgoa-backend-1.0.so.2");
    let binary = std::fs::read(path).map_err(|_| Error::Unavailable)?;
    let config = provider.info().oauth.ok_or(Error::InvalidResponse)?;
    let contains = |needle: &str| {
        binary
            .windows(needle.len())
            .any(|part| part == needle.as_bytes())
    };
    let endpoints_match = match provider {
        Provider::Google => {
            contains(config.client_secret)
                && contains(config.authorization_uri)
                && contains(config.token_uri)
        }
        Provider::Microsoft => {
            contains("https://login.microsoftonline.com/%s/oauth2/v2.0/authorize")
                && contains("https://login.microsoftonline.com/%s/oauth2/v2.0/token")
        }
        _ => false,
    };
    if !contains(config.client_id) || !contains(config.scopes) || !endpoints_match {
        return Err(Error::Unavailable);
    }
    Ok(())
}

impl OAuthHttp for SystemOAuthHttp {
    fn exchange(
        &self,
        config: OAuthConfig,
        code: &Secret,
        verifier: &Secret,
    ) -> Result<Value, Error> {
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .redirects(0)
            .build()
            .post(config.token_uri)
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("code", code.expose()),
                ("client_id", config.client_id),
                ("client_secret", config.client_secret),
                ("redirect_uri", config.redirect_uri),
                ("code_verifier", verifier.expose()),
            ])
            .map_err(|_| Error::SignInFailed)?;
        bounded_json(response)
    }

    fn identity(&self, config: OAuthConfig, access_token: &Secret) -> Result<Value, Error> {
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .redirects(0)
            .build()
            .get(config.identity_uri)
            .set(
                "Authorization",
                &format!("Bearer {}", access_token.expose()),
            )
            .call()
            .map_err(|_| Error::SignInFailed)?;
        bounded_json(response)
    }
}

fn bounded_json(response: ureq::Response) -> Result<Value, Error> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Network)?;
    if bytes.len() > 128 * 1024 {
        return Err(Error::InvalidResponse);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
}

impl OAuthAttempt {
    pub fn new(provider: Provider) -> Result<Self, Error> {
        let config = provider.info().oauth.ok_or(Error::InvalidResponse)?;
        let state = random_secret()?;
        let verifier = random_secret()?;
        let digest = Sha256::digest(verifier.expose().as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        let mut authorization_url =
            Url::parse(config.authorization_uri).map_err(|_| Error::InvalidResponse)?;
        authorization_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", config.client_id)
            .append_pair("redirect_uri", config.redirect_uri)
            .append_pair("scope", config.scopes)
            .append_pair("state", state.expose())
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if provider == Provider::Google {
            authorization_url
                .query_pairs_mut()
                .append_pair("access_type", "offline");
        }
        let mut flow = SignIn::default();
        flow.select_provider(provider)
            .map_err(|_| Error::InvalidResponse)?;
        flow.begin_oauth(state)
            .map_err(|_| Error::InvalidResponse)?;
        Ok(Self {
            provider,
            verifier,
            flow,
            authorization_url,
        })
    }

    /// Open this URL through `rmac_portal::open_uri` only after owning the
    /// temporary GOA callback name. Never log the URL: it contains `state`.
    pub fn authorization_url(&self) -> &Url {
        &self.authorization_url
    }

    fn accepts_callback(&self, client_id: &str, uri: &str) -> bool {
        self.flow.clone().browser_return(client_id, uri).is_ok()
    }

    /// Acquire `OAuthReceiver` before calling this so the redirect cannot
    /// arrive before the callback name is owned.
    pub async fn open_in_browser(&self) -> Result<(), Error> {
        rmac_portal::open_uri(self.authorization_url.as_str())
            .await
            .map_err(|_| Error::Unavailable)
    }

    pub fn complete(
        mut self,
        client_id: &str,
        uri: &str,
        http: &impl OAuthHttp,
    ) -> Result<Tokens, Error> {
        let code = self
            .flow
            .browser_return(client_id, uri)
            .map_err(|_| Error::InvalidResponse)?;
        let config = self.provider.info().oauth.ok_or(Error::InvalidResponse)?;
        let response = http.exchange(config, &code, &self.verifier)?;
        let access_token = response
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(Error::InvalidResponse)?;
        let refresh_token = response
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| Secret::new(value.to_owned()));
        let expires_in = response
            .get("expires_in")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0 && *value <= 86_400 * 365)
            .ok_or(Error::InvalidResponse)?;
        let access_token = Secret::new(access_token.to_owned());
        let profile = http.identity(config, &access_token)?;
        let identity = match self.provider {
            Provider::Google => profile.get("email").and_then(Value::as_str),
            Provider::Microsoft => profile.get("mail").and_then(Value::as_str),
            _ => None,
        }
        .filter(|value| !value.is_empty())
        .ok_or(Error::InvalidResponse)?;
        let presentation_identity = if self.provider == Provider::Microsoft {
            profile
                .get("userPrincipalName")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(identity)
        } else {
            identity
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::InvalidResponse)?;
        let expires_at =
            (now.as_micros() as i64).saturating_add(expires_in.saturating_mul(1_000_000));
        Ok(Tokens {
            access_token,
            refresh_token,
            expires_at,
            identity: identity.to_owned(),
            presentation_identity: presentation_identity.to_owned(),
        })
    }
}

fn random_secret() -> Result<Secret, Error> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| Error::Unavailable)?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes)))
}

#[cfg(target_os = "linux")]
pub mod callback {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    use zbus::blocking::Connection;
    use zbus::fdo::{RequestNameFlags, RequestNameReply};

    use crate::{oauth::OAuthAttempt, Error};

    const NAME: &str = "org.gnome.OnlineAccounts.OAuth2";
    const PATH: &str = "/org/gnome/OnlineAccounts/OAuth2";

    struct Callback {
        sender: Sender<(String, String)>,
    }

    #[zbus::interface(name = "org.gnome.OnlineAccounts.OAuth2")]
    impl Callback {
        fn response(&self, client_id: &str, uri: &str) -> zbus::fdo::Result<()> {
            self.sender
                .send((client_id.to_owned(), uri.to_owned()))
                .map_err(|_| zbus::fdo::Error::Failed("sign-in is no longer active".into()))
        }
    }

    pub struct OAuthReceiver {
        connection: Connection,
        receiver: Receiver<(String, String)>,
    }

    impl OAuthReceiver {
        pub fn begin(attempt: &OAuthAttempt) -> Result<Self, Error> {
            super::verify_installed_provider(attempt.provider)?;
            let connection = Connection::session().map_err(|_| Error::Unavailable)?;
            let (sender, receiver) = mpsc::channel();
            connection
                .object_server()
                .at(PATH, Callback { sender })
                .map_err(|_| Error::Unavailable)?;
            let reply = connection
                .request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into())
                .map_err(|_| Error::Unavailable)?;
            if reply != RequestNameReply::PrimaryOwner {
                return Err(Error::Busy);
            }
            Ok(Self {
                connection,
                receiver,
            })
        }

        /// Waits without polling and discards callbacks with a wrong client,
        /// redirect or state. The deadline is fixed even under spurious calls.
        pub fn receive_for(&self, attempt: &OAuthAttempt) -> Result<(String, String), Error> {
            let deadline = Instant::now() + Duration::from_secs(300);
            for _ in 0..16 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let (client, uri) = self
                    .receiver
                    .recv_timeout(remaining)
                    .map_err(|_| Error::SignInFailed)?;
                if attempt.accepts_callback(&client, &uri) {
                    return Ok((client, uri));
                }
            }
            Err(Error::InvalidResponse)
        }
    }

    impl Drop for OAuthReceiver {
        fn drop(&mut self) {
            let _ = self.connection.release_name(NAME);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fake;
    impl OAuthHttp for Fake {
        fn exchange(
            &self,
            _config: OAuthConfig,
            code: &Secret,
            verifier: &Secret,
        ) -> Result<Value, Error> {
            assert_eq!(code.expose(), "planted-code");
            assert!(!verifier.expose().is_empty());
            Ok(
                json!({"access_token":"planted-token","refresh_token":"planted-refresh","expires_in":3600}),
            )
        }
        fn identity(&self, _config: OAuthConfig, token: &Secret) -> Result<Value, Error> {
            assert_eq!(token.expose(), "planted-token");
            Ok(json!({"email":"planted@example.com"}))
        }
    }

    #[test]
    fn pkce_state_and_exchange_keep_secrets_out_of_debug() {
        let attempt = OAuthAttempt::new(Provider::Google).unwrap();
        let url = attempt.authorization_url().clone();
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "code_challenge_method")
                .unwrap()
                .1,
            "S256"
        );
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        assert!(!format!("{attempt:?}").contains(&state));
        let config = Provider::Google.info().oauth.unwrap();
        let redirect = format!("{}?state={state}&code=planted-code", config.redirect_uri);
        let tokens = attempt
            .complete(config.client_id, &redirect, &Fake)
            .unwrap();
        let debug = format!("{tokens:?}");
        for secret in ["planted-token", "planted-refresh", "planted@example.com"] {
            assert!(!debug.contains(secret));
        }
        assert!(tokens.expires_at > 0);
    }

    struct FakeMicrosoft;
    impl OAuthHttp for FakeMicrosoft {
        fn exchange(
            &self,
            _config: OAuthConfig,
            _code: &Secret,
            _verifier: &Secret,
        ) -> Result<Value, Error> {
            Ok(
                json!({"access_token":"planted-token","refresh_token":"planted-refresh","expires_in":3600}),
            )
        }
        fn identity(&self, _config: OAuthConfig, _token: &Secret) -> Result<Value, Error> {
            Ok(json!({"mail":"alias@example.com","userPrincipalName":"tenant@example.org"}))
        }
    }

    #[test]
    fn microsoft_keeps_goa_identity_and_presentation_distinct() {
        let attempt = OAuthAttempt::new(Provider::Microsoft).unwrap();
        let state = attempt
            .authorization_url()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let config = Provider::Microsoft.info().oauth.unwrap();
        assert!(!attempt.accepts_callback(
            config.client_id,
            &format!("{}?state=wrong&code=secret", config.redirect_uri)
        ));
        let redirect = format!("{}?state={state}&code=planted-code", config.redirect_uri);
        let tokens = attempt
            .complete(config.client_id, &redirect, &FakeMicrosoft)
            .unwrap();
        assert_eq!(tokens.identity, "alias@example.com");
        assert_eq!(tokens.presentation_identity, "tenant@example.org");
    }

    /// Laptop only: `cargo test -p rmac-accounts-linux -- --ignored
    /// installed_goa_provider_table_matches_pinned_clients`.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the reference laptop's installed libgoa-backend"]
    fn installed_goa_provider_table_matches_pinned_clients() {
        verify_installed_provider(Provider::Google).unwrap();
        verify_installed_provider(Provider::Microsoft).unwrap();
    }
}
