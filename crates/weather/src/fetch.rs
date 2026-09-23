//! HTTPS requests through the system `curl` (a package dependency of
//! rmac-apps): TLS, proxies and certificates follow the system's own
//! configuration and no TLS stack is compiled into rmac.

use std::io::Read as _;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchError {
    /// No route to the service: DNS, connection or timeout failures.
    Offline,
    /// The service answered with an error or an oversized body.
    Service,
    /// curl itself is missing or could not run.
    Unavailable,
}

impl FetchError {
    /// curl exit codes that mean "not connected" rather than "broken".
    pub fn from_curl_exit(code: Option<i32>) -> Self {
        match code {
            Some(5 | 6 | 7 | 28 | 35 | 45 | 52 | 55 | 56) => Self::Offline,
            _ => Self::Service,
        }
    }
}

pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TIMEOUT_SECONDS: &str = "15";

/// GET an https URL; blocks, so call it off the UI thread.
pub fn get(url: &str) -> Result<Vec<u8>, FetchError> {
    if !url.starts_with("https://") {
        return Err(FetchError::Service);
    }
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            TIMEOUT_SECONDS,
            "--max-filesize",
            "2097152",
            "--user-agent",
            "rmac-weather/0.1 (+https://github.com/snehacodex/rmac)",
            "--",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| FetchError::Unavailable)?;
    let mut body = Vec::new();
    let read = child.stdout.take().map(|stdout| {
        stdout
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
    });
    let status = child.wait().map_err(|_| FetchError::Unavailable)?;
    if !status.success() {
        return Err(FetchError::from_curl_exit(status.code()));
    }
    if !matches!(read, Some(Ok(_))) || body.len() > MAX_RESPONSE_BYTES {
        return Err(FetchError::Service);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_distinguish_offline_from_errors() {
        assert_eq!(FetchError::from_curl_exit(Some(6)), FetchError::Offline);
        assert_eq!(FetchError::from_curl_exit(Some(7)), FetchError::Offline);
        assert_eq!(FetchError::from_curl_exit(Some(28)), FetchError::Offline);
        assert_eq!(FetchError::from_curl_exit(Some(22)), FetchError::Service);
        assert_eq!(FetchError::from_curl_exit(None), FetchError::Service);
    }

    #[test]
    fn only_https_is_fetched() {
        assert_eq!(get("http://example.com"), Err(FetchError::Service));
        assert_eq!(get("file:///etc/passwd"), Err(FetchError::Service));
    }
}
