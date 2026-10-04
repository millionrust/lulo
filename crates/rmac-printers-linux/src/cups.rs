//! Read-only CUPS queries over the scheduler's local socket.
//!
//! Listing printers and jobs needs no authorisation, so these go straight
//! to cupsd as IPP over HTTP on its domain socket (`/run/cups/cups.sock`,
//! or `CUPS_SERVER` when that names a socket, as in the private test
//! session). Nothing here changes CUPS; changes go through cups-pk-helper
//! ([`crate::admin`]).

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ipp::{self, Request, TAG_KEYWORD, TAG_NAME, TAG_URI};
use crate::model::{Job, Printer};
use crate::Error;

const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

pub struct Cups {
    socket: PathBuf,
}

impl Cups {
    /// The local scheduler.
    pub fn local() -> Self {
        let socket = std::env::var_os("CUPS_SERVER")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                ["/run/cups/cups.sock", "/var/run/cups/cups.sock"]
                    .into_iter()
                    .map(PathBuf::from)
                    .find(|path| path.exists())
            })
            .unwrap_or_else(|| PathBuf::from("/run/cups/cups.sock"));
        Self { socket }
    }

    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    fn call(&self, body: Vec<u8>) -> Result<ipp::Response, Error> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|_| Error::Unavailable)?;
        stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
            .map_err(|_| Error::Unavailable)?;
        let header = format!(
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(header.as_bytes())
            .and_then(|()| stream.write_all(&body))
            .map_err(|_| Error::Unavailable)?;
        let mut response = Vec::new();
        stream
            .take(MAX_RESPONSE)
            .read_to_end(&mut response)
            .map_err(|_| Error::Unavailable)?;
        let body = http_body(&response)?;
        ipp::parse(&body).map_err(|_| Error::Failed)
    }

    /// Every printer queue (classes left out), sorted by display name.
    pub fn printers(&self) -> Result<Vec<Printer>, Error> {
        let mut request = Request::new(ipp::CUPS_GET_PRINTERS, 1);
        request.keywords(
            "requested-attributes",
            &[
                "printer-name",
                "printer-info",
                "printer-location",
                "printer-make-and-model",
                "printer-state",
                "printer-state-reasons",
                "printer-is-accepting-jobs",
                "printer-is-shared",
                "printer-type",
                "device-uri",
            ],
        );
        let response = self.call(request.finish())?;
        if response.status == 0x0406 {
            // client-error-not-found: no printers at all.
            return Ok(Vec::new());
        }
        if !response.is_success() {
            return Err(Error::Failed);
        }
        let mut printers: Vec<Printer> = response
            .groups_tagged(0x04)
            .filter_map(Printer::from_group)
            .filter(|printer| !printer.is_class)
            .collect();
        printers.sort_by(|left, right| {
            left.display_name()
                .to_lowercase()
                .cmp(&right.display_name().to_lowercase())
        });
        Ok(printers)
    }

    /// The scheduler's (system-wide) default queue, if one is set.
    pub fn server_default(&self) -> Result<Option<String>, Error> {
        let mut request = Request::new(ipp::CUPS_GET_DEFAULT, 2);
        request.keywords("requested-attributes", &["printer-name"]);
        let response = self.call(request.finish())?;
        if !response.is_success() {
            return Ok(None);
        }
        Ok(response
            .groups_tagged(0x04)
            .find_map(|group| group.text("printer-name").map(str::to_owned)))
    }

    /// The unfinished jobs of one printer, oldest first.
    pub fn jobs(&self, printer: &str) -> Result<Vec<Job>, Error> {
        if !crate::model::validate_printer_name(printer) {
            return Err(Error::Failed);
        }
        let mut request = Request::new(ipp::GET_JOBS, 3);
        request
            .attribute(
                TAG_URI,
                "printer-uri",
                &format!("ipp://localhost/printers/{printer}"),
            )
            .attribute(TAG_NAME, "requesting-user-name", &user_name())
            .attribute(TAG_KEYWORD, "which-jobs", "not-completed")
            .keywords(
                "requested-attributes",
                &[
                    "job-id",
                    "job-name",
                    "job-originating-user-name",
                    "job-state",
                    "job-k-octets",
                ],
            );
        let response = self.call(request.finish())?;
        if response.status == 0x0406 {
            return Ok(Vec::new());
        }
        if !response.is_success() {
            return Err(Error::Failed);
        }
        let mut jobs: Vec<Job> = response
            .groups_tagged(0x02)
            .filter_map(Job::from_group)
            .collect();
        jobs.sort_by_key(|job| job.id);
        Ok(jobs)
    }
}

fn user_name() -> String {
    std::env::var("USER")
        .ok()
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_graphic()))
        .unwrap_or_else(|| "anonymous".into())
}

/// The body of an HTTP/1.1 response: status 200 required, then
/// Content-Length, chunked, or read-to-close framing.
fn http_body(response: &[u8]) -> Result<Vec<u8>, Error> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(Error::Failed)?;
    let head = std::str::from_utf8(&response[..split]).map_err(|_| Error::Failed)?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(Error::Failed)?;
    match status {
        200 => {}
        401 | 403 => return Err(Error::NotAuthorized),
        _ => return Err(Error::Failed),
    }
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        }
    }
    let body = &response[split + 4..];
    if chunked {
        return dechunk(body);
    }
    match length {
        Some(length) => body.get(..length).map(<[u8]>::to_vec).ok_or(Error::Failed),
        None => Ok(body.to_vec()),
    }
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(Error::Failed)?;
        let size_text = std::str::from_utf8(&body[..line_end]).map_err(|_| Error::Failed)?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| Error::Failed)?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        let chunk = body.get(..size).ok_or(Error::Failed)?;
        out.extend_from_slice(chunk);
        body = body.get(size + 2..).ok_or(Error::Failed)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::tests::ResponseBuilder;
    use crate::ipp::{TAG_ENUM, TAG_INTEGER, TAG_TEXT};
    use std::os::unix::net::UnixListener;

    #[test]
    fn http_framing_is_decoded() {
        let ipp = ResponseBuilder::new(0).finish();
        let mut plain = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
            ipp.len()
        )
        .into_bytes();
        plain.extend_from_slice(&ipp);
        assert_eq!(http_body(&plain).unwrap(), ipp);

        let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        let (first, second) = ipp.split_at(5);
        chunked.extend_from_slice(format!("{:x}\r\n", first.len()).as_bytes());
        chunked.extend_from_slice(first);
        chunked.extend_from_slice(b"\r\n");
        chunked.extend_from_slice(format!("{:X};ext=1\r\n", second.len()).as_bytes());
        chunked.extend_from_slice(second);
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(http_body(&chunked).unwrap(), ipp);

        assert_eq!(
            http_body(b"HTTP/1.1 403 Forbidden\r\n\r\n"),
            Err(Error::NotAuthorized)
        );
        assert_eq!(http_body(b"garbage"), Err(Error::Failed));
        assert_eq!(
            http_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"),
            Err(Error::Failed)
        );
    }

    /// A one-shot fake cupsd: answers each connection with the next canned
    /// IPP body and records the request bodies it received.
    fn fake_cupsd(responses: Vec<Vec<u8>>) -> (PathBuf, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let socket = std::env::temp_dir().join(format!(
            "rmac-fake-cupsd-{}-{}.sock",
            std::process::id(),
            responses.len()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..split]).to_string();
                        let length: usize = head
                            .lines()
                            .find_map(|line| line.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= split + 4 + length {
                            requests.push(request[split + 4..].to_vec());
                            break;
                        }
                    }
                    if read == 0 {
                        break;
                    }
                }
                let mut reply = b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
                reply.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
                reply.extend_from_slice(&body);
                reply.extend_from_slice(b"\r\n0\r\n\r\n");
                stream.write_all(&reply).unwrap();
            }
            requests
        });
        (socket, handle)
    }

    #[test]
    fn printers_and_jobs_come_from_the_scheduler_socket() {
        let printers = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Zeta")
            .int(TAG_ENUM, "printer-state", 3)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Alpha_Laser")
            .text(TAG_TEXT, "printer-info", "Alpha Laser")
            .int(TAG_ENUM, "printer-state", 4)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "AllPrinters")
            .int(TAG_ENUM, "printer-type", 0x0001)
            .finish();
        let default = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Zeta")
            .finish();
        let jobs = ResponseBuilder::new(0)
            .group(0x02)
            .int(TAG_INTEGER, "job-id", 7)
            .text(TAG_NAME, "job-name", "Letter.pdf")
            .int(TAG_ENUM, "job-state", 3)
            .finish();
        let (socket, server) = fake_cupsd(vec![printers, default, jobs]);
        let cups = Cups::at(&socket);
        let listed = cups.printers().unwrap();
        let names: Vec<_> = listed.iter().map(Printer::display_name).collect();
        assert_eq!(names, ["Alpha Laser", "Zeta"]);
        assert_eq!(listed[0].status_label(), "Printing");
        assert_eq!(cups.server_default().unwrap().as_deref(), Some("Zeta"));
        let queue = cups.jobs("Zeta").unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].name, "Letter.pdf");
        assert_eq!(queue[0].state.label(), "Waiting");
        let requests = server.join().unwrap();
        assert_eq!(&requests[0][2..4], &ipp::CUPS_GET_PRINTERS.to_be_bytes());
        assert_eq!(&requests[2][2..4], &ipp::GET_JOBS.to_be_bytes());
        assert!(String::from_utf8_lossy(&requests[2]).contains("ipp://localhost/printers/Zeta"));
        let _ = std::fs::remove_file(socket);
    }

    #[test]
    fn a_missing_scheduler_is_unavailable_and_bad_names_never_reach_it() {
        let cups = Cups::at("/nonexistent/cups.sock");
        assert_eq!(cups.printers(), Err(Error::Unavailable));
        assert_eq!(cups.jobs("../etc"), Err(Error::Failed));
    }
}
