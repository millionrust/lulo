//! The one HTTPS boundary of the Graph backend. Production uses ureq over
//! rustls with the platform verifier; tests replay recorded responses
//! through the same trait, so no test touches the network.

use std::{sync::Arc, time::Duration};

use rmac_accounts::Secret;
use ureq::tls::{RootCerts, TlsConfig};

/// Every request goes to this service root.
pub const GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";
/// Server-provided paging and delta links must stay under this origin; the
/// bearer token is never sent anywhere else.
const GRAPH_ORIGIN: &str = "https://graph.microsoft.com/";

/// JSON replies are small pages; anything larger is refused unread.
pub const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
/// A whole RFC 5322 message, attachments included (Exchange's own send
/// limit is 150 MB for MIME through Graph uploads; ordinary sends are 35 MB).
pub const MAX_MIME_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Patch,
    Delete,
}

pub struct Request {
    pub method: Method,
    /// Always an absolute `https://graph.microsoft.com/` URL; see `graph_link`.
    pub url: String,
    pub content_type: Option<&'static str>,
    pub body: Vec<u8>,
    /// Extra `Prefer` preferences besides the immutable-id one every request
    /// carries (for example `odata.maxpagesize=100`).
    pub prefer: Option<&'static str>,
    pub max_body: u64,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Bodies can hold a whole message; never print them.
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The request never left this machine (no route, DNS, connect, TLS).
    NotSent,
    /// The connection failed after the request may have reached Microsoft.
    Interrupted,
    /// The reply was larger than the request's limit or unreadable.
    Body,
}

/// `token` is the account's OAuth access token from GOA. Implementations
/// put it only in the `Authorization` header and must never log it.
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: &Request, token: &Secret) -> Result<Response, HttpError>;
}

/// Accepts a link only if it points at Microsoft Graph over HTTPS, so a
/// tampered or unexpected `@odata.nextLink` cannot redirect the token.
pub fn graph_link(url: &str) -> Option<String> {
    let valid = url.starts_with(GRAPH_ORIGIN)
        && url.len() <= 8 * 1024
        && !url
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte < 0x20);
    valid.then(|| url.to_owned())
}

/// Percent-encodes one path segment (Graph ids are opaque and may contain
/// `=`, `+` or `/`).
pub fn segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// ureq over rustls (ring) with the platform certificate verifier: TLS is
/// always verified, plain HTTP and redirects are refused.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_resolve(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(120)))
            .max_redirects(0)
            .https_only(true)
            .http_status_as_error(false)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .unversioned_rustls_crypto_provider(Arc::new(
                        rustls::crypto::ring::default_provider(),
                    ))
                    .build(),
            )
            .build()
            .new_agent();
        Self { agent }
    }
}

fn classify(error: &ureq::Error) -> HttpError {
    match error {
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Tls(_)
        | ureq::Error::RequireHttpsOnly(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => {
            HttpError::NotSent
        }
        ureq::Error::BodyExceedsLimit(_) => HttpError::Body,
        _ => HttpError::Interrupted,
    }
}

impl HttpTransport for UreqTransport {
    fn send(&self, request: &Request, token: &Secret) -> Result<Response, HttpError> {
        if graph_link(&request.url).is_none() {
            return Err(HttpError::NotSent);
        }
        let authorization = format!("Bearer {}", token.expose());
        let prefer = match request.prefer {
            Some(extra) => format!("IdType=\"ImmutableId\", {extra}"),
            None => "IdType=\"ImmutableId\"".to_owned(),
        };
        let url = request.url.as_str();
        let result = match request.method {
            Method::Get => self
                .agent
                .get(url)
                .header("Authorization", &authorization)
                .header("Prefer", &prefer)
                .call(),
            Method::Delete => self
                .agent
                .delete(url)
                .header("Authorization", &authorization)
                .header("Prefer", &prefer)
                .call(),
            Method::Post | Method::Patch => {
                let builder = if request.method == Method::Post {
                    self.agent.post(url)
                } else {
                    self.agent.patch(url)
                };
                builder
                    .header("Authorization", &authorization)
                    .header("Prefer", &prefer)
                    .header(
                        "Content-Type",
                        request.content_type.unwrap_or("application/json"),
                    )
                    .send(request.body.as_slice())
            }
        };
        let mut response = result.map_err(|error| classify(&error))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(request.max_body)
            .read_to_vec()
            .map_err(|error| match classify(&error) {
                HttpError::NotSent => HttpError::Interrupted,
                other => other,
            })?;
        Ok(Response { status, body })
    }
}
