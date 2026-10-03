//! Blocking IMAP transport for a dedicated mail worker thread.
//! No method should be called from the UI thread. Credentials and server text
//! are intentionally absent from errors and logs.

mod protocol;
mod transport;

use protocol::{
    parse_capabilities, parse_copyuid, parse_list, parse_select, parse_uid_fetch, quote, Response,
};
pub use protocol::{
    Capabilities, CopyUid, Mailbox, MailboxKind, MessageChange, SelectState, SyncCursor,
};
use std::{fmt, io, time::Duration};
pub use transport::Interrupt;
use transport::Transport;
use zeroize::Zeroize;

const MAX_IDLE: Duration = Duration::from_secs(25 * 60);

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsMode {
    Implicit,
    StartTls,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub timeout: Duration,
}

impl Config {
    pub fn implicit(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 993,
            tls: TlsMode::Implicit,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Authentication<'a> {
    XOAuth2 { user: &'a str, token: &'a Secret },
    OAuthBearer { user: &'a str, token: &'a Secret },
    Plain { user: &'a str, password: &'a Secret },
    Login { user: &'a str, password: &'a Secret },
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls,
    Protocol(&'static str),
    Rejected(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => f.write_str("IMAP connection failed"),
            Self::Tls => f.write_str("IMAP TLS verification failed"),
            Self::Protocol(message) | Self::Rejected(message) | Self::Unsupported(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// A single TLS-protected IMAP connection. Move it to a worker thread.
pub struct Client {
    transport: Transport,
    next_tag: u32,
    capabilities: Capabilities,
}

impl Client {
    pub fn connect(config: &Config) -> Result<Self, Error> {
        let certs = rustls_native_certs::load_native_certs();
        if certs.certs.is_empty() {
            return Err(Error::Tls);
        }
        let mut roots = rustls::RootCertStore::empty();
        for cert in certs.certs {
            let _ = roots.add(cert);
        }
        Self::connect_with_roots(config, roots)
    }

    /// Useful for private servers whose CA is supplied by the caller.
    pub fn connect_with_roots(
        config: &Config,
        roots: rustls::RootCertStore,
    ) -> Result<Self, Error> {
        let mut transport = Transport::connect(config, roots)?;
        if config.tls == TlsMode::Implicit {
            let greeting = transport.read_response()?;
            if !greeting.first_line().starts_with(b"* OK ") {
                return Err(Error::Protocol("IMAP server did not send an OK greeting"));
            }
        }
        let mut client = Self {
            transport,
            next_tag: 1,
            capabilities: Capabilities::default(),
        };
        client.capabilities = parse_capabilities(&client.command("CAPABILITY")?);
        Ok(client)
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn interrupt_handle(&self) -> io::Result<Interrupt> {
        self.transport.interrupt_handle()
    }

    pub fn authenticate(&mut self, auth: Authentication<'_>) -> Result<(), Error> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        if let Authentication::Login { user, password } = auth {
            if self.capabilities.has("LOGINDISABLED") {
                return Err(Error::Unsupported("IMAP server does not allow LOGIN"));
            }
            validate_auth_field(user)?;
            validate_auth_field(password.expose())?;
            let mut command = format!(
                "LOGIN {} {}",
                protocol::quote_string(user)?,
                protocol::quote_string(password.expose())?
            );
            let result = self.command(&command);
            command.zeroize();
            result?;
            return self.enable_extensions();
        }
        let (mechanism, payload) = match auth {
            Authentication::XOAuth2 { user, token } => {
                if !self.capabilities.has("AUTH=XOAUTH2") {
                    return Err(Error::Unsupported("IMAP server does not support XOAUTH2"));
                }
                validate_auth_field(user)?;
                validate_auth_field(token.expose())?;
                (
                    "XOAUTH2",
                    format!("user={user}\x01auth=Bearer {}\x01\x01", token.expose()),
                )
            }
            Authentication::OAuthBearer { user, token } => {
                if !self.capabilities.has("AUTH=OAUTHBEARER") {
                    return Err(Error::Unsupported(
                        "IMAP server does not support OAUTHBEARER",
                    ));
                }
                validate_auth_field(user)?;
                validate_auth_field(token.expose())?;
                let user = user.replace('=', "=3D").replace(',', "=2C");
                (
                    "OAUTHBEARER",
                    format!("n,a={user},\x01auth=Bearer {}\x01\x01", token.expose()),
                )
            }
            Authentication::Plain { user, password } => {
                if !self.capabilities.has("AUTH=PLAIN") {
                    return Err(Error::Unsupported("IMAP server does not support PLAIN"));
                }
                validate_auth_field(user)?;
                if password
                    .expose()
                    .chars()
                    .any(|ch| matches!(ch, '\0' | '\r' | '\n'))
                {
                    return Err(Error::Protocol("Invalid IMAP credential"));
                }
                ("PLAIN", format!("\0{user}\0{}", password.expose()))
            }
            Authentication::Login { .. } => unreachable!(),
        };
        let encoded = STANDARD.encode(payload);
        let tag = self.tag();
        if self.capabilities.has("SASL-IR") {
            self.transport
                .write_line(&format!("{tag} AUTHENTICATE {mechanism} {encoded}"))?;
        } else {
            self.transport
                .write_line(&format!("{tag} AUTHENTICATE {mechanism}"))?;
            let challenge = self.transport.read_response()?;
            if !challenge.first_line().starts_with(b"+") {
                return Err(Error::Rejected("IMAP authentication rejected"));
            }
            self.transport.write_line(&encoded)?;
        }
        self.collect_auth(&tag)?;
        self.enable_extensions()
    }

    fn enable_extensions(&mut self) -> Result<(), Error> {
        // Servers may advertise a different capability set after sign-in.
        self.capabilities = parse_capabilities(&self.command("CAPABILITY")?);
        // ENABLE is connection-scoped; QRESYNC implicitly enables CONDSTORE.
        if self.capabilities.has("QRESYNC") {
            self.command("ENABLE QRESYNC")?;
        } else if self.capabilities.has("CONDSTORE") {
            self.command("ENABLE CONDSTORE")?;
        }
        Ok(())
    }

    pub fn list_mailboxes(&mut self) -> Result<Vec<Mailbox>, Error> {
        let cmd = if self.capabilities.has("SPECIAL-USE") && self.capabilities.has("LIST-EXTENDED")
        {
            "LIST \"\" \"*\" RETURN (SPECIAL-USE)"
        } else {
            "LIST \"\" \"*\""
        };
        Ok(parse_list(&self.command(cmd)?))
    }

    pub fn select(
        &mut self,
        mailbox: &str,
        cursor: Option<&SyncCursor>,
    ) -> Result<SelectState, Error> {
        let mut cmd = format!("SELECT {}", quote(mailbox)?);
        if let Some(cursor) = cursor {
            if self.capabilities.has("QRESYNC")
                && cursor.uid_validity > 0
                && cursor.highest_modseq > 0
            {
                cmd.push_str(&format!(
                    " (QRESYNC ({} {}))",
                    cursor.uid_validity, cursor.highest_modseq
                ));
            } else if self.capabilities.has("CONDSTORE") {
                cmd.push_str(" (CONDSTORE)");
            }
        } else if self.capabilities.has("CONDSTORE") {
            cmd.push_str(" (CONDSTORE)");
        }
        let responses = self.command(&cmd)?;
        parse_select(&responses)
    }

    /// Returns flag/modseq changes. A server without CONDSTORE receives a full UID fetch.
    pub fn fetch_changes(&mut self, since: Option<u64>) -> Result<Vec<MessageChange>, Error> {
        let modseq = self.capabilities.has("CONDSTORE") || self.capabilities.has("QRESYNC");
        let mut cmd = if modseq {
            "UID FETCH 1:* (UID FLAGS MODSEQ)".to_string()
        } else {
            "UID FETCH 1:* (UID FLAGS)".to_string()
        };
        if modseq {
            if let Some(since) = since.filter(|value| *value > 0) {
                cmd.push_str(&format!(" (CHANGEDSINCE {since})"));
            }
        }
        Ok(parse_uid_fetch(&self.command(&cmd)?))
    }

    /// Fetches one complete RFC 5322 message, without setting the Seen flag.
    pub fn fetch_body(&mut self, uid: u32) -> Result<Option<Vec<u8>>, Error> {
        let responses = self.command(&format!("UID FETCH {uid} (UID BODY.PEEK[])"))?;
        Ok(responses.iter().find_map(Response::first_literal))
    }

    pub fn move_uids(
        &mut self,
        uid_set: &str,
        destination: &str,
    ) -> Result<Option<CopyUid>, Error> {
        if !self.capabilities.has("MOVE") {
            return Err(Error::Unsupported("IMAP server does not support MOVE"));
        }
        protocol::validate_uid_set(uid_set)?;
        let (_, completion) =
            self.command_with_completion(&format!("UID MOVE {uid_set} {}", quote(destination)?))?;
        Ok(parse_copyuid(&completion))
    }

    pub fn expunge_uids(&mut self, uid_set: &str) -> Result<(), Error> {
        if !self.capabilities.has("UIDPLUS") {
            return Err(Error::Unsupported("IMAP server does not support UIDPLUS"));
        }
        protocol::validate_uid_set(uid_set)?;
        self.command(&format!("UID EXPUNGE {uid_set}"))?;
        Ok(())
    }

    /// Replace standard flags for one UID. The caller must verify UIDVALIDITY
    /// before replaying a queued change.
    pub fn store_flags(&mut self, uid: u32, flags: &[&str]) -> Result<(), Error> {
        if uid == 0
            || flags.iter().any(|flag| {
                !matches!(
                    *flag,
                    "\\Seen" | "\\Answered" | "\\Flagged" | "\\Draft" | "\\Deleted"
                )
            })
        {
            return Err(Error::Protocol("Invalid IMAP flags"));
        }
        self.command(&format!(
            "UID STORE {uid} FLAGS.SILENT ({})",
            flags.join(" ")
        ))?;
        Ok(())
    }

    /// Waits for one unsolicited update or at most 25 minutes, then exits IDLE.
    /// The owning worker may re-issue IDLE or close the connection.
    pub fn idle_once(&mut self, limit: Duration) -> Result<Option<String>, Error> {
        if !self.capabilities.has("IDLE") {
            return Err(Error::Unsupported("IMAP server does not support IDLE"));
        }
        let tag = self.tag();
        self.transport.write_line(&format!("{tag} IDLE"))?;
        let continuation = self.transport.read_response()?;
        if !continuation.first_line().starts_with(b"+") {
            return Err(Error::Rejected("IMAP IDLE rejected"));
        }
        self.transport.set_read_timeout(limit.min(MAX_IDLE))?;
        let event = match self.transport.read_response() {
            Ok(response) => Some(response.safe_event()),
            Err(Error::Io(ref error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        self.transport.reset_read_timeout()?;
        self.transport.write_line("DONE")?;
        self.collect(&tag)?;
        Ok(event)
    }

    fn command(&mut self, command: &str) -> Result<Vec<Response>, Error> {
        self.command_with_completion(command)
            .map(|(responses, _)| responses)
    }

    fn command_with_completion(
        &mut self,
        command: &str,
    ) -> Result<(Vec<Response>, Response), Error> {
        let tag = self.tag();
        self.transport.write_line(&format!("{tag} {command}"))?;
        self.collect_with_completion(&tag)
    }

    fn collect(&mut self, tag: &str) -> Result<Vec<Response>, Error> {
        self.collect_with_completion(tag)
            .map(|(responses, _)| responses)
    }

    fn collect_with_completion(&mut self, tag: &str) -> Result<(Vec<Response>, Response), Error> {
        let mut responses = Vec::new();
        loop {
            let response = self.transport.read_response()?;
            let first = response.first_line();
            if first.starts_with(tag.as_bytes()) && first.get(tag.len()) == Some(&b' ') {
                if first.get(tag.len() + 1..tag.len() + 3) == Some(b"OK") {
                    return Ok((responses, response));
                }
                return Err(Error::Rejected("IMAP command rejected"));
            }
            if first.starts_with(b"* BYE") {
                return Err(Error::Rejected("IMAP server closed the connection"));
            }
            responses.push(response);
        }
    }

    fn collect_auth(&mut self, tag: &str) -> Result<(), Error> {
        let mut challenge_answered = false;
        loop {
            let response = self.transport.read_response()?;
            let first = response.first_line();
            if first.starts_with(b"+") {
                if challenge_answered {
                    return Err(Error::Rejected("IMAP authentication rejected"));
                }
                // XOAUTH2 failures carry a base64 JSON challenge. It can
                // contain token details, so discard it without logging.
                self.transport.write_line("")?;
                challenge_answered = true;
                continue;
            }
            if first.starts_with(tag.as_bytes()) && first.get(tag.len()) == Some(&b' ') {
                return if first.get(tag.len() + 1..tag.len() + 3) == Some(b"OK") {
                    Ok(())
                } else {
                    Err(Error::Rejected("IMAP authentication rejected"))
                };
            }
            if first.starts_with(b"* BYE") {
                return Err(Error::Rejected("IMAP server closed the connection"));
            }
        }
    }

    fn tag(&mut self) -> String {
        let tag = format!("L{:08}", self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        tag
    }
}

fn validate_auth_field(value: &str) -> Result<(), Error> {
    if value.chars().any(char::is_control) {
        Err(Error::Protocol("Invalid IMAP credential"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
