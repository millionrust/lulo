//! Change the caller's own login password by driving `passwd(1)` over a
//! private pseudo-terminal, the way GNOME's Users panel does.
//!
//! Going through `passwd` means PAM verifies the current password and
//! applies the system's password-quality rules; no administrator
//! authorisation is involved and nothing runs with more privilege than
//! `passwd`'s own setuid helper. The passwords are written to the
//! terminal only after `passwd` has switched echo off, so they never come
//! back through the transcript; the transcript itself is zeroized.
//!
//! The environment is cleared (`LC_ALL=C` for stable prompts) and the
//! program path is absolute, so `PATH` and locale cannot redirect it.

use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::model::Secret;

/// The system `passwd`.
pub const PASSWD: &str = "/usr/bin/passwd";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasswdError {
    /// PAM rejected the current password.
    WrongPassword,
    /// PAM refused the new password; the message is PAM's own reason
    /// (password-quality rules, minimum age), never a secret.
    Rejected(String),
    /// `passwd` is missing or a pseudo-terminal could not be opened.
    Unavailable,
    /// `passwd` stopped responding.
    TimedOut,
    Failed,
}

impl PasswdError {
    pub fn message(&self) -> String {
        match self {
            PasswdError::WrongPassword => "The old password you entered isn’t correct.".into(),
            PasswdError::Rejected(reason) if !reason.is_empty() => {
                format!("The new password can’t be used. {reason}")
            }
            PasswdError::Rejected(_) => {
                "The new password can’t be used. Choose a different password.".into()
            }
            PasswdError::Unavailable => "Your password can’t be changed on this computer.".into(),
            PasswdError::TimedOut => "Changing the password took too long. Try again.".into(),
            PasswdError::Failed => "Your password couldn’t be changed. Try again.".into(),
        }
    }
}

/// Change the signed-in user's password with the system `passwd`.
pub fn change_own_password(current: &Secret, new: &Secret) -> Result<(), PasswdError> {
    change_password_with(Path::new(PASSWD), current, new, TIMEOUT)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Prompt {
    Current,
    New,
    Retype,
}

fn classify(line: &str) -> Option<Prompt> {
    let line = line.trim().to_ascii_lowercase();
    if !line.ends_with(':') || !line.contains("password") {
        return None;
    }
    if line.contains("retype") || line.contains("re-enter") || line.contains("again") {
        Some(Prompt::Retype)
    } else if line.contains("new") {
        Some(Prompt::New)
    } else {
        // "Current password:", "(current) UNIX password:", "Password:".
        Some(Prompt::Current)
    }
}

/// PAM's reason from a "BAD PASSWORD: …" or "passwd: …" line, if any.
fn reason(transcript: &str) -> Option<String> {
    transcript.lines().rev().find_map(|line| {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("BAD PASSWORD:") {
            return Some(sentence(rest));
        }
        let rest = line.strip_prefix("passwd:")?.trim();
        let lower = rest.to_ascii_lowercase();
        (!lower.contains("updated successfully")
            && !lower.contains("unchanged")
            && !lower.contains("manipulation error")
            && !lower.contains("authentication"))
        .then(|| sentence(rest))
    })
}

fn sentence(text: &str) -> String {
    let text: String = text
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let mut chars = text.chars();
    let mut out: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// [`change_own_password`] with an explicit program and timeout, for tests.
pub fn change_password_with(
    program: &Path,
    current: &Secret,
    new: &Secret,
    timeout: Duration,
) -> Result<(), PasswdError> {
    if current.expose().contains(['\n', '\r', '\0']) || new.expose().contains(['\n', '\r', '\0']) {
        return Err(PasswdError::Failed);
    }
    if new.is_empty() {
        return Err(PasswdError::Rejected(String::new()));
    }
    if !program.is_absolute() || !program.is_file() {
        return Err(PasswdError::Unavailable);
    }
    let (master, slave) = open_pty().map_err(|_| PasswdError::Unavailable)?;
    let mut child = {
        let stdin = slave.try_clone().map_err(|_| PasswdError::Unavailable)?;
        let stdout = slave.try_clone().map_err(|_| PasswdError::Unavailable)?;
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls (setsid, ioctl) run between
        // fork and exec; they make the pseudo-terminal the controlling
        // terminal so PAM's conversation reads from it.
        unsafe {
            use std::os::unix::process::CommandExt as _;
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn().map_err(|_| PasswdError::Unavailable)?
        // `command` (and with it the parent's copies of the slave) drops
        // here, so the master sees end-of-file when passwd exits.
    };
    let result = converse(&master, &mut child, current, new, timeout);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|_| PasswdError::Failed);
    let (transcript, finished) = result?;
    if status?.success() && finished {
        return Ok(());
    }
    let lower = transcript.to_ascii_lowercase();
    if lower.contains("manipulation error") || lower.contains("authentication failure") {
        Err(PasswdError::WrongPassword)
    } else {
        Err(PasswdError::Rejected(
            reason(&transcript).unwrap_or_default(),
        ))
    }
}

fn open_pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    // SAFETY: openpty writes two descriptors into the provided integers; the
    // name, termios and winsize pointers may be null.
    let status = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty succeeded, so both are fresh descriptors we own.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        // SAFETY: fcntl on a descriptor we own. The child gets the slave
        // through dup2 in Command, which clears close-on-exec on 0/1/2.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok((master, slave))
}

/// Answer passwd's prompts. Returns the (secret-free) transcript and
/// whether every prompt was answered exactly once.
fn converse(
    master: &OwnedFd,
    child: &mut Child,
    current: &Secret,
    new: &Secret,
    timeout: Duration,
) -> Result<(Zeroizing<String>, bool), PasswdError> {
    let deadline = Instant::now() + timeout;
    let mut transcript = Zeroizing::new(String::new());
    let mut pending = Zeroizing::new(String::new());
    let mut answered: Vec<Prompt> = Vec::new();
    let mut buffer = Zeroizing::new([0_u8; 1024]);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(PasswdError::TimedOut);
        }
        let mut poll = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = remaining.as_millis().min(1000) as libc::c_int;
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(PasswdError::Failed);
        }
        if ready == 0 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            continue;
        }
        // SAFETY: reading into our own buffer from a descriptor we own.
        let read =
            unsafe { libc::read(master.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            // EIO once the child closed the slave: end of conversation.
            break;
        }
        let text = String::from_utf8_lossy(&buffer[..read as usize]).into_owned();
        let text = Zeroizing::new(text);
        transcript.push_str(&text);
        pending.push_str(&text);
        let Some(last_line) = pending.rsplit('\n').next().map(str::to_owned) else {
            continue;
        };
        let Some(prompt) = classify(&last_line) else {
            continue;
        };
        pending.clear();
        let reply = match prompt {
            Prompt::Current if answered.is_empty() => current,
            Prompt::New if !answered.contains(&Prompt::New) => new,
            Prompt::Retype
                if answered.contains(&Prompt::New) && !answered.contains(&Prompt::Retype) =>
            {
                new
            }
            // Asked again: PAM rejected an answer and is retrying. Stop
            // rather than guess; the transcript says why.
            _ => {
                let _ = child.kill();
                let lower = transcript.to_ascii_lowercase();
                return if prompt == Prompt::Current {
                    Err(PasswdError::WrongPassword)
                } else if lower.contains("do not match") {
                    Err(PasswdError::Failed)
                } else {
                    Err(PasswdError::Rejected(
                        reason(&transcript).unwrap_or_default(),
                    ))
                };
            }
        };
        wait_for_echo_off(master, deadline);
        let mut line = Zeroizing::new(Vec::with_capacity(reply.expose().len() + 1));
        line.extend_from_slice(reply.expose().as_bytes());
        line.push(b'\n');
        write_all(master, &line)?;
        answered.push(prompt);
    }
    let finished = answered.contains(&Prompt::Current)
        && answered.contains(&Prompt::New)
        && answered.contains(&Prompt::Retype);
    Ok((transcript, finished))
}

/// PAM prints the prompt, then turns echo off (flushing pending input).
/// Writing before that would lose the answer or echo it back, so wait
/// until the terminal reports echo off.
fn wait_for_echo_off(master: &OwnedFd, deadline: Instant) {
    let wait_until = deadline.min(Instant::now() + Duration::from_secs(2));
    while Instant::now() < wait_until {
        // SAFETY: termios is plain data; tcgetattr fills it.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr on our own pseudo-terminal master.
        if unsafe { libc::tcgetattr(master.as_raw_fd(), &mut termios) } != 0 {
            return;
        }
        if termios.c_lflag & libc::ECHO == 0 {
            // Let the flush that accompanies the change finish.
            std::thread::sleep(Duration::from_millis(30));
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn write_all(master: &OwnedFd, mut bytes: &[u8]) -> Result<(), PasswdError> {
    while !bytes.is_empty() {
        // SAFETY: writing from a live slice to a descriptor we own.
        let written =
            unsafe { libc::write(master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if written < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(PasswdError::Failed);
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_classified_like_pam_prints_them() {
        assert_eq!(classify("Current password: "), Some(Prompt::Current));
        assert_eq!(classify("(current) UNIX password:"), Some(Prompt::Current));
        assert_eq!(classify("New password: "), Some(Prompt::New));
        assert_eq!(classify("Retype new password: "), Some(Prompt::Retype));
        assert_eq!(classify("Changing password for amy."), None);
        assert_eq!(classify("passwd: password updated successfully"), None);
    }

    #[test]
    fn pam_reasons_are_kept_and_secrets_are_not_involved() {
        assert_eq!(
            reason("BAD PASSWORD: The password is shorter than 8 characters\nNew password: "),
            Some("The password is shorter than 8 characters.".into())
        );
        assert_eq!(
            reason("passwd: Authentication token manipulation error\npasswd: password unchanged"),
            None
        );
        assert_eq!(
            reason("You must wait longer to change your password\npasswd: Authentication token manipulation error"),
            None
        );
    }

    #[test]
    fn relative_or_missing_programs_are_refused() {
        let secret = |value: &str| Secret::new(value.into());
        assert_eq!(
            change_password_with(
                Path::new("passwd"),
                &secret("a"),
                &secret("b"),
                Duration::from_secs(1)
            ),
            Err(PasswdError::Unavailable)
        );
        assert_eq!(
            change_password_with(
                Path::new("/nonexistent/passwd"),
                &secret("a"),
                &secret("b"),
                Duration::from_secs(1)
            ),
            Err(PasswdError::Unavailable)
        );
        assert_eq!(
            change_password_with(
                Path::new("/bin/sh"),
                &secret("a\nb"),
                &secret("b"),
                Duration::from_secs(1)
            ),
            Err(PasswdError::Failed)
        );
    }

    /// A stand-in for passwd(1) that prompts with echo off the same way,
    /// accepts "old secret" as the current password and refuses short new
    /// ones with PAM's wording.
    #[cfg(target_os = "linux")]
    fn fake_passwd(dir: &Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let script = dir.join("fake-passwd");
        std::fs::write(
            &script,
            r#"#!/bin/bash
echo "Changing password for tester."
read -r -s -p "Current password: " current; echo
if [ "$current" != "old secret" ]; then
  sleep 0.2
  echo "passwd: Authentication token manipulation error"
  echo "passwd: password unchanged"
  exit 10
fi
read -r -s -p "New password: " new; echo
if [ ${#new} -lt 8 ]; then
  echo "BAD PASSWORD: The password is shorter than 8 characters"
  read -r -s -p "New password: " new; echo
  exit 10
fi
read -r -s -p "Retype new password: " again; echo
if [ "$new" != "$again" ]; then
  echo "Sorry, passwords do not match."
  echo "passwd: Authentication token manipulation error"
  exit 10
fi
echo "$new" > "$(dirname "$0")/stored"
echo "passwd: password updated successfully"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        script
    }

    #[cfg(target_os = "linux")]
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rmac-users-passwd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_correct_current_password_changes_it() {
        let dir = scratch("ok");
        let program = fake_passwd(&dir);
        let result = change_password_with(
            &program,
            &Secret::new("old secret".into()),
            &Secret::new("new secret 123".into()),
            Duration::from_secs(10),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            std::fs::read_to_string(dir.join("stored")).unwrap().trim(),
            "new secret 123"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_wrong_current_password_is_reported_as_such() {
        let dir = scratch("wrong");
        let program = fake_passwd(&dir);
        let result = change_password_with(
            &program,
            &Secret::new("guess".into()),
            &Secret::new("new secret 123".into()),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(PasswdError::WrongPassword));
        assert!(!dir.join("stored").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_weak_new_password_reports_pams_reason() {
        let dir = scratch("weak");
        let program = fake_passwd(&dir);
        let result = change_password_with(
            &program,
            &Secret::new("old secret".into()),
            &Secret::new("short".into()),
            Duration::from_secs(10),
        );
        assert_eq!(
            result,
            Err(PasswdError::Rejected(
                "The password is shorter than 8 characters.".into()
            ))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
