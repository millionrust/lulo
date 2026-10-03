//! Worker-thread SMTP submission. The caller triggers `drain_outbox` on
//! connectivity changes; this crate has no timers, polling loop or UI work.

use base64::{engine::general_purpose::STANDARD, Engine};
use rmac_mail_storage::{MailStorage, OutboxMessage, OutboxState};
use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    sync::Arc,
    time::Duration,
};

pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
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

pub enum Authentication {
    Xoauth2 { user: String, token: Secret },
    Plain { user: String, password: Secret },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    StartTls,
    ImplicitTls,
}

pub struct Config {
    pub host: String,
    pub port: u16,
    pub helo_name: String,
    pub security: Security,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls,
    MissingStartTls,
    AuthenticationUnavailable,
    InvalidEnvelope,
    Protocol,
    Rejected(u16),
    Storage(rmac_mail_storage::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => write!(f, "SMTP connection failed"),
            Self::Tls => write!(f, "SMTP TLS failed"),
            Self::MissingStartTls => write!(f, "SMTP server does not offer STARTTLS"),
            Self::AuthenticationUnavailable => write!(f, "SMTP authentication method unavailable"),
            Self::InvalidEnvelope => write!(f, "invalid SMTP envelope"),
            Self::Protocol => write!(f, "invalid SMTP response"),
            Self::Rejected(code) => write!(f, "SMTP server rejected submission ({code})"),
            Self::Storage(_) => write!(f, "Outbox storage failed"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rmac_mail_storage::Error> for Error {
    fn from(value: rmac_mail_storage::Error) -> Self {
        Self::Storage(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Safe,
    Hold,
}
pub struct SubmissionFailure {
    pub error: Error,
    pub retry: Retry,
}

trait IoStream: Read + Write {}
impl<T: Read + Write> IoStream for T {}
type Stream = Box<dyn IoStream + Send>;

struct Session {
    reader: BufReader<Stream>,
}
impl Session {
    fn new(stream: Stream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }
    fn response(&mut self) -> Result<(u16, String), Error> {
        let mut capabilities = String::new();
        let mut first_code = None;
        for _ in 0..100 {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(Error::Protocol);
            }
            let bytes = line.as_bytes();
            if bytes.len() < 5
                || !bytes[0..3].iter().all(u8::is_ascii_digit)
                || !matches!(bytes[3], b' ' | b'-')
                || !line.ends_with('\n')
            {
                return Err(Error::Protocol);
            }
            let code = line[0..3].parse::<u16>().map_err(|_| Error::Protocol)?;
            if let Some(first) = first_code {
                if first != code {
                    return Err(Error::Protocol);
                }
            } else {
                first_code = Some(code);
            }
            // Capabilities are only used after EHLO. Never expose the server
            // response in errors because it may echo private addresses.
            capabilities.push_str(&line[4..].to_ascii_uppercase());
            if bytes[3] == b' ' {
                return Ok((code, capabilities));
            }
        }
        Err(Error::Protocol)
    }
    fn command(&mut self, command: &str, expected: u16) -> Result<String, Error> {
        self.reader.get_mut().write_all(command.as_bytes())?;
        self.reader.get_mut().write_all(b"\r\n")?;
        self.reader.get_mut().flush()?;
        let (code, text) = self.response()?;
        if code != expected {
            return Err(Error::Rejected(code));
        }
        Ok(text)
    }
    fn into_inner(self) -> Stream {
        self.reader.into_inner()
    }
}

fn tls(stream: Stream, host: &str) -> Result<Stream, Error> {
    let certs = rustls_native_certs::load_native_certs();
    if certs.certs.is_empty() {
        return Err(Error::Tls);
    }
    let roots = RootCertStore::from_iter(certs.certs);
    let config = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| Error::Tls)?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let name = ServerName::try_from(host.to_owned()).map_err(|_| Error::Tls)?;
    let connection = ClientConnection::new(config, name).map_err(|_| Error::Tls)?;
    Ok(Box::new(StreamOwned::new(connection, stream)))
}

fn valid_address(value: &str) -> bool {
    !value.is_empty()
        && value.is_ascii()
        && value.matches('@').count() == 1
        && !value
            .bytes()
            .any(|byte| byte <= 32 || byte == 127 || matches!(byte, b'<' | b'>' | b',' | b';'))
}

fn auth(
    session: &mut Session,
    capabilities: &str,
    authentication: &Authentication,
) -> Result<(), Error> {
    match authentication {
        Authentication::Xoauth2 { user, token } => {
            if !capabilities.lines().any(|line| {
                line.starts_with("AUTH ")
                    && line.split_whitespace().any(|method| method == "XOAUTH2")
            }) {
                return Err(Error::AuthenticationUnavailable);
            }
            if !valid_address(user) {
                return Err(Error::InvalidEnvelope);
            }
            let challenge =
                STANDARD.encode(format!("user={user}\x01auth=Bearer {}\x01\x01", token.0));
            session.command(&format!("AUTH XOAUTH2 {challenge}"), 235)?;
        }
        Authentication::Plain { user, password } => {
            if !capabilities.lines().any(|line| {
                line.starts_with("AUTH ") && line.split_whitespace().any(|method| method == "PLAIN")
            }) {
                return Err(Error::AuthenticationUnavailable);
            }
            if !valid_address(user) {
                return Err(Error::InvalidEnvelope);
            }
            let challenge = STANDARD.encode(format!("\0{user}\0{}", password.0));
            session.command(&format!("AUTH PLAIN {challenge}"), 235)?;
        }
    }
    Ok(())
}

fn send_data(session: &mut Session, bytes: &[u8]) -> Result<(), Error> {
    let stream = session.reader.get_mut();
    let mut at_line_start = true;
    let mut last = None;
    for &byte in bytes {
        if at_line_start && byte == b'.' {
            stream.write_all(b".")?;
        }
        if byte == b'\n' && last != Some(b'\r') {
            stream.write_all(b"\r")?;
        }
        stream.write_all(&[byte])?;
        at_line_start = byte == b'\n';
        last = Some(byte);
    }
    if last != Some(b'\n') {
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b".\r\n")?;
    stream.flush()?;
    Ok(())
}

/// Send one message using a TLS connection. Run only on a worker thread.
/// A failed final DATA acknowledgement is held for review: the server may
/// have accepted the message before the connection failed.
pub fn submit(
    config: &Config,
    authentication: &Authentication,
    message: &OutboxMessage,
) -> Result<(), SubmissionFailure> {
    let fail = |error| {
        let retry = match error {
            Error::Rejected(code) if code >= 500 => Retry::Hold,
            Error::Tls
            | Error::MissingStartTls
            | Error::AuthenticationUnavailable
            | Error::InvalidEnvelope
            | Error::Protocol => Retry::Hold,
            _ => Retry::Safe,
        };
        SubmissionFailure { error, retry }
    };
    if !valid_address(&message.envelope_from)
        || message.recipients.is_empty()
        || !message
            .recipients
            .iter()
            .all(|address| valid_address(address))
        || config.helo_name.bytes().any(|b| b <= 32 || b == 127)
    {
        return Err(fail(Error::InvalidEnvelope));
    }
    let socket = TcpStream::connect((config.host.as_str(), config.port))
        .map_err(|error| fail(Error::Io(error)))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| fail(Error::Io(error)))?;
    socket
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| fail(Error::Io(error)))?;
    let stream: Stream = Box::new(socket);
    let mut session = Session::new(if config.security == Security::ImplicitTls {
        tls(stream, &config.host).map_err(fail)?
    } else {
        stream
    });
    let (greeting, _) = session.response().map_err(fail)?;
    if greeting != 220 {
        return Err(fail(Error::Rejected(greeting)));
    }
    let mut caps = session
        .command(&format!("EHLO {}", config.helo_name), 250)
        .map_err(fail)?;
    if config.security == Security::StartTls {
        if !caps.lines().any(|line| line.trim() == "STARTTLS") {
            return Err(fail(Error::MissingStartTls));
        }
        session.command("STARTTLS", 220).map_err(fail)?;
        session = Session::new(tls(session.into_inner(), &config.host).map_err(fail)?);
        caps = session
            .command(&format!("EHLO {}", config.helo_name), 250)
            .map_err(fail)?;
    }
    auth(&mut session, &caps, authentication).map_err(fail)?;
    session
        .command(&format!("MAIL FROM:<{}>", message.envelope_from), 250)
        .map_err(fail)?;
    for recipient in &message.recipients {
        // RCPT 251 means the server will forward the accepted recipient.
        match session.command(&format!("RCPT TO:<{recipient}>"), 250) {
            Ok(_) | Err(Error::Rejected(251)) => {}
            Err(error) => return Err(fail(error)),
        }
    }
    session.command("DATA", 354).map_err(fail)?;
    send_data(&mut session, &message.bytes).map_err(|error| SubmissionFailure {
        error,
        retry: Retry::Hold,
    })?;
    let (code, _) = session.response().map_err(|error| SubmissionFailure {
        error,
        retry: Retry::Hold,
    })?;
    if code != 250 {
        return Err(SubmissionFailure {
            error: Error::Rejected(code),
            retry: if code >= 500 {
                Retry::Hold
            } else {
                Retry::Safe
            },
        });
    }
    let _ = session.command("QUIT", 221);
    Ok(())
}

/// Drain messages only when called by the runtime after a send request or
/// connectivity event. A failure stops this drain; a later event may retry.
pub fn drain_outbox(
    store: &mut MailStorage,
    config: &Config,
    authentication: &Authentication,
) -> Result<usize, Error> {
    let mut sent = 0;
    while let Some(message) = store.claim_outbox()? {
        match submit(config, authentication, &message) {
            Ok(()) => {
                store.complete_outbox(message.id)?;
                sent += 1;
            }
            Err(failure) => {
                store.update_outbox_state(
                    message.id,
                    if failure.retry == Retry::Safe {
                        OutboxState::Queued
                    } else {
                        OutboxState::Held
                    },
                )?;
                return Err(failure.error);
            }
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    struct Fixture {
        read: io::Cursor<Vec<u8>>,
        written: Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl Read for Fixture {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read.read(buf)
        }
    }
    impl Write for Fixture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn multiline_reply_and_dot_stuffing() {
        let written = Arc::new(std::sync::Mutex::new(Vec::new()));
        let stream = Fixture {
            read: io::Cursor::new(b"250-local\r\n250 AUTH PLAIN XOAUTH2\r\n".to_vec()),
            written: written.clone(),
        };
        let mut session = Session::new(Box::new(stream));
        let (code, caps) = session.response().unwrap();
        assert_eq!(code, 250);
        assert!(caps.contains("AUTH PLAIN XOAUTH2"));
        send_data(&mut session, b"From: a@example.test\r\n\r\n.first\n.second").unwrap();
        assert_eq!(
            *written.lock().unwrap(),
            b"From: a@example.test\r\n\r\n..first\r\n..second\r\n.\r\n"
        );
    }

    #[test]
    fn credentials_are_redacted() {
        let token = Secret::new("planted-secret".into());
        assert_eq!(format!("{token:?} {token}"), "[redacted] [redacted]");
        assert!(!valid_address("a@example.test\r\nRCPT TO:<b@example.test>"));
    }

    #[test]
    fn starttls_is_required_before_authentication() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"220 fixture ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            stream.write_all(b"250 fixture hello\r\n").unwrap();
            let mut rest = String::new();
            reader.read_to_string(&mut rest).unwrap();
            (command, rest)
        });
        let message = OutboxMessage {
            id: 1,
            envelope_from: "a@example.test".into(),
            recipients: vec!["b@example.test".into()],
            bytes: b"body".to_vec(),
        };
        let result = submit(
            &Config {
                host: "127.0.0.1".into(),
                port,
                helo_name: "lulo.test".into(),
                security: Security::StartTls,
            },
            &Authentication::Plain {
                user: "a@example.test".into(),
                password: Secret::new("planted-secret".into()),
            },
            &message,
        );
        assert!(matches!(
            result,
            Err(SubmissionFailure {
                error: Error::MissingStartTls,
                retry: Retry::Hold
            })
        ));
        let (command, rest) = server.join().unwrap();
        assert_eq!(command, "EHLO lulo.test\r\n");
        assert!(rest.is_empty());
    }
}
