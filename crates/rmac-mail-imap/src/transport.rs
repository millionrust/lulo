use crate::{protocol::Response, Config, Error, TlsMode};
use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

const MAX_LINE: usize = 64 * 1024;
const MAX_LITERAL: usize = 64 * 1024 * 1024;

pub(crate) struct Transport {
    stream: StreamOwned<ClientConnection, TcpStream>,
    default_timeout: Duration,
}

/// A control handle that wakes a worker blocked in IDLE without a timer.
pub struct Interrupt(TcpStream);

impl Interrupt {
    pub fn wake(&self) -> io::Result<()> {
        self.0.shutdown(Shutdown::Both)
    }
}

impl Transport {
    pub(crate) fn interrupt_handle(&self) -> io::Result<Interrupt> {
        self.stream.sock.try_clone().map(Interrupt)
    }
    pub(crate) fn connect(config: &Config, roots: RootCertStore) -> Result<Self, Error> {
        let mut socket = None;
        for address in (config.host.as_str(), config.port).to_socket_addrs()? {
            if let Ok(stream) = TcpStream::connect_timeout(&address, config.timeout) {
                socket = Some(stream);
                break;
            }
        }
        let mut socket = socket.ok_or_else(|| {
            Error::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                "IMAP unavailable",
            ))
        })?;
        socket.set_read_timeout(Some(config.timeout))?;
        socket.set_write_timeout(Some(config.timeout))?;
        let server_name = ServerName::try_from(config.host.clone()).map_err(|_| Error::Tls)?;
        let tls_config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );

        if config.tls == TlsMode::StartTls {
            let greeting = read_line(&mut socket)?;
            if !greeting.starts_with(b"* OK ") {
                return Err(Error::Protocol("IMAP server did not send an OK greeting"));
            }
            socket.write_all(b"L00000000 STARTTLS\r\n")?;
            let response = read_line(&mut socket)?;
            if !response.starts_with(b"L00000000 OK ") {
                return Err(Error::Tls);
            }
        }

        let connection = ClientConnection::new(tls_config, server_name).map_err(|_| Error::Tls)?;
        let mut stream = StreamOwned::new(connection, socket);
        stream.flush().map_err(|_| Error::Tls)?;
        Ok(Self {
            stream,
            default_timeout: config.timeout,
        })
    }

    pub(crate) fn write_line(&mut self, line: &str) -> Result<(), Error> {
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\r\n")?;
        self.stream.flush()?;
        Ok(())
    }

    pub(crate) fn read_response(&mut self) -> Result<Response, Error> {
        let mut raw = read_line(&mut self.stream)?;
        let mut literal = None;
        while let Some(length) = trailing_literal(&raw)? {
            if length > MAX_LITERAL {
                return Err(Error::Protocol("IMAP literal is too large"));
            }
            let mut bytes = vec![0; length];
            self.stream.read_exact(&mut bytes)?;
            raw.extend_from_slice(&bytes);
            literal = Some(bytes);
            raw.extend_from_slice(&read_line(&mut self.stream)?);
        }
        Response::new(raw, literal)
    }

    pub(crate) fn set_read_timeout(&mut self, timeout: Duration) -> Result<(), Error> {
        self.stream.sock.set_read_timeout(Some(timeout))?;
        Ok(())
    }

    pub(crate) fn reset_read_timeout(&mut self) -> Result<(), Error> {
        self.set_read_timeout(self.default_timeout)
    }
}

fn read_line(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        if let Err(error) = reader.read_exact(&mut byte) {
            // A timeout after part of a response would otherwise look like an
            // idle timeout and let the caller send DONE into a partial frame.
            return if line.is_empty() {
                Err(error)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Incomplete IMAP response",
                ))
            };
        }
        line.push(byte[0]);
        if line.len() > MAX_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IMAP line is too long",
            ));
        }
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
    }
}

fn trailing_literal(line: &[u8]) -> Result<Option<usize>, Error> {
    let Some(content) = line.strip_suffix(b"\r\n") else {
        return Err(Error::Protocol("Invalid IMAP line ending"));
    };
    if !content.ends_with(b"}") {
        return Ok(None);
    }
    let Some(start) = content.iter().rposition(|byte| *byte == b'{') else {
        return Ok(None);
    };
    let digits = &content[start + 1..content.len() - 1];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    let number = std::str::from_utf8(digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::Protocol("Invalid IMAP literal size"))?;
    Ok(Some(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_idle_frame_cannot_be_mistaken_for_a_clean_timeout() {
        let mut truncated = io::Cursor::new(b"* 2 EXISTS\r");
        assert_eq!(
            read_line(&mut truncated).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
