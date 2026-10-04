//! polkit's own password checker, `polkit-agent-helper-1`, spoken to exactly
//! as libpolkit-agent-1 does (polkit 127 `polkitagentsession.c`):
//!
//! * If `/run/polkit/agent-helper.socket` exists (polkit 126+, Ubuntu 26.04:
//!   the helper is a socket-activated root service and no longer setuid),
//!   connect to it and write `<user>\n<cookie>\n`.
//! * Otherwise spawn the setuid helper as `polkit-agent-helper-1 <user>` and
//!   write `<cookie>\n` on its stdin (never on the command line, where other
//!   processes could read it).
//!
//! The helper then sends `g_strcompress`-escaped lines: `PAM_PROMPT_ECHO_OFF
//! <prompt>` and `PAM_PROMPT_ECHO_ON <prompt>` (answered with one line),
//! `PAM_ERROR_MSG <text>`, `PAM_TEXT_INFO <text>`, and finally `SUCCESS` or
//! `FAILURE`. The helper reports the result to polkitd itself; this process
//! never verifies a password and never learns more than success or failure.
//!
//! Nothing here logs a prompt answer or a PAM message.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::secret::Secret;
use crate::text;

pub const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Where distributions install the helper (`pkg-config --variable
/// libprivdir polkit-agent-1` plus the historical locations).
pub const HELPER_EXECUTABLES: [&str; 4] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/libexec/polkit-1/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

/// Longest helper line accepted; anything longer ends the session.
const MAX_LINE: usize = 4096;

/// How to reach the helper.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HelperConfig {
    pub socket: Option<PathBuf>,
    pub executable: Option<PathBuf>,
}

impl HelperConfig {
    /// The system's helper: the socket when polkit provides one, and the
    /// setuid executable as the fallback.
    pub fn system() -> Self {
        Self {
            socket: Path::new(HELPER_SOCKET)
                .exists()
                .then(|| PathBuf::from(HELPER_SOCKET)),
            executable: HELPER_EXECUTABLES
                .iter()
                .map(PathBuf::from)
                .find(|path| path.is_file()),
        }
    }
}

/// What the helper says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HelperEvent {
    /// PAM wants an answer; `echo` is false for passwords.
    Prompt {
        echo: bool,
        text: String,
    },
    Info(String),
    Error(String),
    /// The conversation ended: `true` when polkitd was told the user
    /// authenticated.
    Finished(bool),
}

enum Answer {
    Response(Secret),
    Cancel,
}

/// One running conversation. Dropping it cancels it.
pub struct HelperSession {
    answers: mpsc::Sender<Answer>,
    wake: UnixStream,
}

impl HelperSession {
    /// Answer the outstanding prompt. The bytes are written once and the
    /// buffer is zeroized.
    pub fn answer(&self, secret: Secret) {
        let _ = self.answers.send(Answer::Response(secret));
    }

    /// End the conversation; the helper sees its input close.
    pub fn cancel(&self) {
        let _ = self.answers.send(Answer::Cancel);
        let _ = (&self.wake).write(&[1]);
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Transport {
    reader: Box<dyn Read + Send>,
    reader_fd: RawFd,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
}

fn valid_field(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.contains(['\n', '\0'])
}

fn connect(config: &HelperConfig, user: &str, cookie: &str) -> io::Result<Transport> {
    if !valid_field(user) || !valid_field(cookie) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid user or cookie",
        ));
    }
    if let Some(socket) = &config.socket {
        match UnixStream::connect(socket) {
            Ok(stream) => {
                let mut writer = stream.try_clone()?;
                writer.write_all(user.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.write_all(cookie.as_bytes())?;
                writer.write_all(b"\n")?;
                let reader_fd = stream.as_raw_fd();
                return Ok(Transport {
                    reader: Box::new(stream),
                    reader_fd,
                    writer: Box::new(writer),
                    child: None,
                });
            }
            Err(error) => {
                eprintln!("polkit helper socket unavailable ({error}); spawning the helper");
            }
        }
    }
    let executable = config
        .executable
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no polkit agent helper"))?;
    let mut child = Command::new(executable)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .spawn()?;
    let mut writer = child.stdin.take().expect("piped stdin");
    let reader = child.stdout.take().expect("piped stdout");
    if let Err(error) = writer
        .write_all(cookie.as_bytes())
        .and_then(|()| writer.write_all(b"\n"))
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let reader_fd = reader.as_raw_fd();
    Ok(Transport {
        reader: Box::new(reader),
        reader_fd,
        writer: Box::new(writer),
        child: Some(child),
    })
}

/// Start a conversation for `user` with polkitd's `cookie`. Events arrive on
/// `on_event` from a dedicated thread, which blocks only in `poll` and in
/// waiting for an answer: no polling loop, no wakeups while the user types.
pub fn start(
    config: &HelperConfig,
    user: &str,
    cookie: &str,
    on_event: impl FnMut(HelperEvent) + Send + 'static,
) -> io::Result<HelperSession> {
    let transport = connect(config, user, cookie)?;
    let (answers, answers_rx) = mpsc::channel();
    let (wake, wake_rx) = UnixStream::pair()?;
    thread::Builder::new()
        .name("polkit-helper".into())
        .spawn(move || converse(transport, answers_rx, wake_rx, on_event))?;
    Ok(HelperSession { answers, wake })
}

enum Line {
    Text(String),
    Closed,
}

/// Wait for a full line or the wake socket. Returns `Closed` on EOF, error,
/// overlong line or cancellation.
fn read_line(transport: &mut Transport, wake: &UnixStream, pending: &mut Vec<u8>) -> Line {
    loop {
        if let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            return Line::Text(line);
        }
        if pending.len() > MAX_LINE {
            return Line::Closed;
        }
        let mut fds = [
            libc::pollfd {
                fd: transport.reader_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` is a valid array of two pollfd structs.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Line::Closed;
        }
        if fds[1].revents != 0 {
            return Line::Closed;
        }
        let mut chunk = [0_u8; 1024];
        match transport.reader.read(&mut chunk) {
            Ok(0) => return Line::Closed,
            Ok(count) => pending.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Line::Closed,
        }
    }
}

/// Parse one helper line (after `g_strcompress`).
pub fn parse_line(raw: &str) -> Option<HelperEvent> {
    let line = text::unescape(raw);
    let message = |rest: &str| text::display(rest, text::MAX_MESSAGE_CHARS);
    if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        Some(HelperEvent::Prompt {
            echo: false,
            text: message(rest),
        })
    } else if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        Some(HelperEvent::Prompt {
            echo: true,
            text: message(rest),
        })
    } else if let Some(rest) = line.strip_prefix("PAM_ERROR_MSG ") {
        Some(HelperEvent::Error(message(rest)))
    } else if let Some(rest) = line.strip_prefix("PAM_TEXT_INFO ") {
        Some(HelperEvent::Info(message(rest)))
    } else if line.starts_with("SUCCESS") {
        Some(HelperEvent::Finished(true))
    } else if line.starts_with("FAILURE") {
        Some(HelperEvent::Finished(false))
    } else {
        None
    }
}

fn converse(
    mut transport: Transport,
    answers: mpsc::Receiver<Answer>,
    wake: UnixStream,
    mut on_event: impl FnMut(HelperEvent),
) {
    let mut pending = Vec::new();
    let mut success = false;
    loop {
        let Line::Text(raw) = read_line(&mut transport, &wake, &mut pending) else {
            break;
        };
        match parse_line(&raw) {
            Some(HelperEvent::Finished(result)) => {
                success = result;
                break;
            }
            Some(event @ HelperEvent::Prompt { .. }) => {
                on_event(event);
                match answers.recv() {
                    Ok(Answer::Response(secret)) => {
                        let written = secret.expose(|bytes| {
                            transport
                                .writer
                                .write_all(bytes)
                                .and_then(|()| transport.writer.write_all(b"\n"))
                                .and_then(|()| transport.writer.flush())
                        });
                        drop(secret);
                        if written.is_err() {
                            break;
                        }
                    }
                    Ok(Answer::Cancel) | Err(_) => break,
                }
            }
            Some(event) => on_event(event),
            None => break,
        }
    }
    pending.fill(0);
    // Closing our ends tells the helper the conversation is over.
    let Transport {
        reader,
        writer,
        child,
        ..
    } = transport;
    drop(writer);
    drop(reader);
    if let Some(mut child) = child {
        // A setuid helper cannot be signalled; it exits on the closed pipes.
        let _ = child.kill();
        let _ = child.wait();
    }
    on_event(HelperEvent::Finished(success));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_helper_line() {
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_OFF Password: "),
            Some(HelperEvent::Prompt {
                echo: false,
                text: "Password:".into()
            })
        );
        assert_eq!(
            parse_line(r"PAM_PROMPT_ECHO_ON Code\t1"),
            Some(HelperEvent::Prompt {
                echo: true,
                text: "Code 1".into()
            })
        );
        assert_eq!(
            parse_line("PAM_ERROR_MSG Account locked"),
            Some(HelperEvent::Error("Account locked".into()))
        );
        assert_eq!(
            parse_line(r"PAM_TEXT_INFO Place your finger\n"),
            Some(HelperEvent::Info("Place your finger".into()))
        );
        assert_eq!(parse_line("SUCCESS"), Some(HelperEvent::Finished(true)));
        assert_eq!(parse_line("FAILURE"), Some(HelperEvent::Finished(false)));
        assert_eq!(parse_line("GARBAGE"), None);
    }

    #[test]
    fn rejects_fields_that_would_break_the_line_protocol() {
        let config = HelperConfig::default();
        assert!(connect(&config, "jacob\nroot", "cookie").is_err());
        assert!(connect(&config, "jacob", "").is_err());
        assert_eq!(
            connect(&config, "jacob", "cookie").err().map(|e| e.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }
}
