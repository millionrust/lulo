//! Go ▸ Connect to Server… (⌘K) (FILES-39).
//!
//! The Mac mounts SMB, AFP, NFS, WebDAV and FTP shares at a typed address.
//! On Linux the same job belongs to GVFS: `gio mount URI` mounts the share
//! (SMB, SFTP, FTP, WebDAV, NFS) and GVFS publishes it under
//! `$XDG_RUNTIME_DIR/gvfs`, where Files already browses it. This module is
//! the part that does not need a window: turning what the user typed into a
//! URI, driving `gio mount` through a pseudo-terminal so its name and
//! password prompts can be answered, and finding the folder the mount
//! appeared in.

#[cfg(any(windows, test))]
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;

/// How many servers the address field remembers.
pub(crate) const MAX_RECENT_SERVERS: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AddressError {
    Empty,
    /// A scheme GVFS has no backend for.
    Unsupported(String),
    /// Spaces, control characters, a missing server name.
    Invalid,
}

impl AddressError {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Empty => "Enter the address of a server.".to_owned(),
            Self::Unsupported(scheme) => {
                format!("Files can’t connect to “{scheme}” servers.")
            }
            Self::Invalid => "That server address isn’t valid.".to_owned(),
        }
    }
}

/// What the user typed as the URI `gio mount` takes. A bare name is an SMB
/// server, as on the Mac; a Windows `\\server\share` becomes `smb://`.
pub(crate) fn parse_address(text: &str) -> Result<String, AddressError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AddressError::Empty);
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AddressError::Invalid);
    }
    let (scheme, rest) = if let Some(unc) = text.strip_prefix("\\\\") {
        ("smb".to_owned(), unc.replace('\\', "/"))
    } else if let Some((scheme, rest)) = text.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        let mapped = match scheme.as_str() {
            "smb" | "cifs" => "smb",
            "nfs" => "nfs",
            "sftp" | "ssh" => "sftp",
            "ftp" => "ftp",
            "ftps" => "ftps",
            "dav" | "http" => "dav",
            "davs" | "https" => "davs",
            "afp" => "afp",
            _ => return Err(AddressError::Unsupported(scheme)),
        };
        (mapped.to_owned(), rest.to_owned())
    } else {
        ("smb".to_owned(), text.to_owned())
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty() || host.starts_with('-') {
        return Err(AddressError::Invalid);
    }
    Ok(format!("{scheme}://{rest}"))
}

/// The scheme and host of a URI from [`parse_address`].
pub(crate) fn scheme_and_host(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once("://")?;
    let authority = rest.split('/').next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some((scheme, host))
}

/// The share name in an `smb://server/share/...` URI.
fn first_path_segment(uri: &str) -> Option<&str> {
    let (_, rest) = uri.split_once("://")?;
    let path = rest.split_once('/')?.1;
    path.split('/').next().filter(|segment| !segment.is_empty())
}

/// The Windows form of an SMB address: `smb://server/share` is
/// `\\server\share`. Other schemes have no Windows path.
#[cfg(any(windows, test))]
pub(crate) fn unc_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("smb://")?;
    let rest = rest.rsplit('@').next()?;
    Some(PathBuf::from(format!("\\\\{}", rest.replace('/', "\\"))))
}

/// Which entry of `$XDG_RUNTIME_DIR/gvfs` is the mount just made. GVFS names
/// them `smb-share:server=NAS,share=media`, `sftp:host=example.com,user=me`,
/// `dav:host=...,ssl=false` and so on.
pub(crate) fn pick_mount_dir(entries: &[String], uri: &str) -> Option<String> {
    let (scheme, host) = scheme_and_host(uri)?;
    let prefix = match scheme {
        "smb" => "smb-share",
        "sftp" => "sftp",
        "ftp" | "ftps" => "ftp",
        "dav" | "davs" => "dav",
        "nfs" => "nfs",
        "afp" => "afp-volume",
        _ => return None,
    };
    let host_key = if scheme == "smb" { "server" } else { "host" };
    let share = (scheme == "smb").then(|| first_path_segment(uri)).flatten();
    let candidates: Vec<&String> = entries
        .iter()
        .filter(|entry| {
            let Some(parameters) = entry.strip_prefix(prefix).and_then(|r| r.strip_prefix(':'))
            else {
                return false;
            };
            parameters.split(',').any(|pair| {
                pair.split_once('=')
                    .is_some_and(|(key, value)| key == host_key && value.eq_ignore_ascii_case(host))
            })
        })
        .collect();
    if let Some(share) = share {
        if let Some(exact) = candidates.iter().find(|entry| {
            entry.split(',').any(|pair| {
                pair.rsplit_once("share=")
                    .is_some_and(|(_, value)| value.eq_ignore_ascii_case(share))
            })
        }) {
            return Some((*exact).clone());
        }
    }
    candidates.first().map(|entry| (*entry).clone())
}

/// Add `uri` to a most-recent-first list, once, capped at `limit`.
pub(crate) fn remember(list: &mut Vec<String>, uri: &str, limit: usize) {
    list.retain(|existing| existing != uri);
    list.insert(0, uri.to_owned());
    list.truncate(limit);
}

/// What `gio mount` asks for while mounting.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    Anonymous,
    User,
    Domain,
    Password,
}

/// The question on the unfinished last line of `gio mount`'s output, if it
/// ends in one ("User [me]: ", "Password: ", "Anonymous? (y/n) [n]: ").
#[cfg(any(target_os = "linux", test))]
pub(crate) fn classify_prompt(transcript: &str) -> Option<Prompt> {
    let line = transcript.rsplit(['\n', '\r']).next()?;
    let line = line.trim_end();
    if !line.ends_with(':') {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    if lower.contains("anonymous") {
        Some(Prompt::Anonymous)
    } else if lower.contains("password") {
        Some(Prompt::Password)
    } else if lower.contains("domain") {
        Some(Prompt::Domain)
    } else if lower.contains("user") {
        Some(Prompt::User)
    } else {
        None
    }
}

/// Byte offset where the last (possibly unfinished) line of `transcript`
/// starts.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn last_line_start(transcript: &str) -> usize {
    transcript.rfind(['\n', '\r']).map_or(0, |index| index + 1)
}

/// How the user wants to sign in.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub(crate) guest: bool,
    pub(crate) user: String,
    pub(crate) password: String,
}

#[cfg(any(target_os = "linux", test))]
impl Credentials {
    /// The line to type for `prompt`. A domain is left at its default.
    pub(crate) fn answer(&self, prompt: Prompt) -> String {
        match prompt {
            Prompt::Anonymous => if self.guest { "y\n" } else { "n\n" }.to_owned(),
            Prompt::User if self.guest => "guest\n".to_owned(),
            Prompt::User => format!("{}\n", self.user),
            Prompt::Domain => "\n".to_owned(),
            Prompt::Password if self.guest => "\n".to_owned(),
            Prompt::Password => format!("{}\n", self.password),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum MountOutcome {
    Mounted,
    /// The server wants a name and password; the text says for what.
    NeedsCredentials(String),
    Failed(String),
}

/// Reduce `gio mount`'s final output to one line a person can read.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn failure_message(transcript: &str) -> String {
    let line = transcript
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("The server did not respond.");
    line.strip_prefix("gio: ").unwrap_or(line).to_owned()
}

/// True when the output says the share was already mounted, which is
/// success for Connect to Server.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn already_mounted(transcript: &str) -> bool {
    transcript
        .to_ascii_lowercase()
        .contains("location is already mounted")
}

/// Stop a running [`mount`] (the Cancel button).
#[cfg(target_os = "linux")]
pub(crate) fn cancel(pid_slot: &AtomicI32) {
    use std::sync::atomic::Ordering;
    let pid = pid_slot.load(Ordering::Acquire);
    if pid > 0 {
        // SAFETY: `kill` only sends a signal to a process this module
        // started and has not yet reaped (the slot is cleared before wait).
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

/// Mount `uri` with `gio mount`, answering its prompts from `credentials`.
/// Blocks until it finishes, so call it off the UI thread. With no
/// credentials the first question that needs one stops the attempt with
/// [`MountOutcome::NeedsCredentials`].
///
/// `gio mount` only asks for a name and password when it is attached to a
/// terminal, so it runs on a pseudo-terminal whose master end this reads;
/// nothing is polled, the read simply blocks until it prints or exits.
#[cfg(target_os = "linux")]
pub(crate) fn mount(
    uri: &str,
    credentials: Option<&Credentials>,
    pid_slot: &AtomicI32,
) -> MountOutcome {
    use std::io::{Read as _, Write as _};
    use std::os::fd::{FromRawFd as _, OwnedFd};
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;

    let mut master_fd: libc::c_int = -1;
    let mut slave_fd: libc::c_int = -1;
    // SAFETY: both out-pointers are valid; the optional arguments are null.
    let opened = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if opened != 0 {
        return MountOutcome::Failed("Files could not start the connection.".to_owned());
    }
    // SAFETY: `openpty` returned two fresh descriptors that nothing else owns.
    let (master, slave) = unsafe {
        (
            OwnedFd::from_raw_fd(master_fd),
            OwnedFd::from_raw_fd(slave_fd),
        )
    };
    let (stdin, stdout, stderr) = match (slave.try_clone(), slave.try_clone()) {
        (Ok(stdin), Ok(stdout)) => (stdin, stdout, slave),
        _ => return MountOutcome::Failed("Files could not start the connection.".to_owned()),
    };
    let mut command = Command::new("gio");
    command
        .arg("mount")
        .arg(uri)
        .env("LC_ALL", "C.UTF-8")
        .env("LANGUAGE", "C")
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let spawned = command.spawn();
    // The child holds its own copies; drop ours so the master sees EOF when
    // it exits.
    drop(command);
    let mut child = match spawned {
        Ok(child) => child,
        Err(_) => {
            return MountOutcome::Failed(
                "Files could not run gio; install the gvfs package to connect to servers."
                    .to_owned(),
            )
        }
    };
    pid_slot.store(child.id() as i32, Ordering::Release);

    let mut master = std::fs::File::from(master);
    let mut transcript = String::new();
    let mut buffer = [0u8; 1024];
    let mut answered_upto = 0usize;
    let mut answered: Vec<Prompt> = Vec::new();
    let mut outcome = None;
    // Reads until end of output (EIO once the child closes the terminal) or
    // an error.
    while let Ok(read @ 1..) = master.read(&mut buffer) {
        transcript.push_str(&String::from_utf8_lossy(&buffer[..read]));
        let Some(prompt) = classify_prompt(&transcript) else {
            continue;
        };
        if last_line_start(&transcript) < answered_upto {
            continue;
        }
        let Some(credentials) = credentials else {
            let message = transcript
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.ends_with(':'))
                .unwrap_or("The server requires a name and password.")
                .to_owned();
            outcome = Some(MountOutcome::NeedsCredentials(message));
            break;
        };
        // Asked for the same thing twice: what was sent was refused.
        if answered.contains(&prompt) && matches!(prompt, Prompt::User | Prompt::Password) {
            outcome = Some(MountOutcome::Failed(
                "The name or password is incorrect.".to_owned(),
            ));
            break;
        }
        if master
            .write_all(credentials.answer(prompt).as_bytes())
            .is_err()
        {
            break;
        }
        answered.push(prompt);
        answered_upto = transcript.len();
    }
    if outcome.is_some() {
        cancel(pid_slot);
    }
    pid_slot.store(0, Ordering::Release);
    let status = child.wait();
    if let Some(outcome) = outcome {
        return outcome;
    }
    match status {
        Ok(status) if status.success() => MountOutcome::Mounted,
        _ if already_mounted(&transcript) => MountOutcome::Mounted,
        _ => MountOutcome::Failed(failure_message(&transcript)),
    }
}

/// Mounting needs GVFS, which only the Linux desktop has.
#[cfg(not(target_os = "linux"))]
pub(crate) fn mount(
    _uri: &str,
    _credentials: Option<&Credentials>,
    _pid_slot: &AtomicI32,
) -> MountOutcome {
    MountOutcome::Failed("Connecting to servers isn’t available on this system.".to_owned())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn cancel(_pid_slot: &AtomicI32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_is_an_smb_server() {
        assert_eq!(
            parse_address("nas.local/media").unwrap(),
            "smb://nas.local/media"
        );
        assert_eq!(parse_address("  nas  ").unwrap(), "smb://nas");
    }

    #[test]
    fn schemes_map_to_what_gvfs_mounts() {
        assert_eq!(parse_address("ssh://me@host").unwrap(), "sftp://me@host");
        assert_eq!(
            parse_address("HTTPS://cloud.example/remote.php/dav").unwrap(),
            "davs://cloud.example/remote.php/dav"
        );
        assert_eq!(
            parse_address("ftp://ftp.example").unwrap(),
            "ftp://ftp.example"
        );
        assert_eq!(parse_address("cifs://nas/x").unwrap(), "smb://nas/x");
    }

    #[test]
    fn windows_unc_paths_become_smb_addresses() {
        assert_eq!(
            parse_address("\\\\nas\\media\\films").unwrap(),
            "smb://nas/media/films"
        );
    }

    #[test]
    fn bad_addresses_are_refused_with_a_reason() {
        assert_eq!(parse_address("   "), Err(AddressError::Empty));
        assert_eq!(parse_address("nas share"), Err(AddressError::Invalid));
        assert_eq!(parse_address("smb:///share"), Err(AddressError::Invalid));
        assert_eq!(parse_address("-oProxy=x"), Err(AddressError::Invalid));
        assert_eq!(
            parse_address("gopher://old.example"),
            Err(AddressError::Unsupported("gopher".to_owned()))
        );
        assert!(parse_address("gopher://old.example")
            .unwrap_err()
            .message()
            .contains("gopher"));
    }

    #[test]
    fn host_survives_user_and_port() {
        assert_eq!(
            scheme_and_host("sftp://me@host.example:2222/home"),
            Some(("sftp", "host.example"))
        );
    }

    #[test]
    fn unc_form_drops_credentials() {
        assert_eq!(
            unc_path("smb://me@nas/media/films"),
            Some(PathBuf::from("\\\\nas\\media\\films"))
        );
        assert_eq!(unc_path("sftp://nas/x"), None);
    }

    #[test]
    fn the_mount_is_found_among_gvfs_entries() {
        let entries = vec![
            "smb-share:server=nas,share=photos".to_owned(),
            "smb-share:server=nas,share=media".to_owned(),
            "sftp:host=example.com,user=me".to_owned(),
        ];
        assert_eq!(
            pick_mount_dir(&entries, "smb://NAS/media").as_deref(),
            Some("smb-share:server=nas,share=media")
        );
        assert_eq!(
            pick_mount_dir(&entries, "smb://nas").as_deref(),
            Some("smb-share:server=nas,share=photos")
        );
        assert_eq!(
            pick_mount_dir(&entries, "sftp://me@example.com/home").as_deref(),
            Some("sftp:host=example.com,user=me")
        );
        assert_eq!(pick_mount_dir(&entries, "ftp://example.com"), None);
        assert_eq!(pick_mount_dir(&[], "smb://nas"), None);
    }

    #[test]
    fn recent_servers_are_unique_newest_first_and_capped() {
        let mut list = vec!["smb://a".to_owned(), "smb://b".to_owned()];
        remember(&mut list, "smb://b", 3);
        assert_eq!(list, ["smb://b", "smb://a"]);
        remember(&mut list, "smb://c", 3);
        remember(&mut list, "smb://d", 3);
        assert_eq!(list, ["smb://d", "smb://c", "smb://b"]);
    }

    #[test]
    fn prompts_are_recognised_only_on_an_unfinished_last_line() {
        assert_eq!(classify_prompt("Password: "), Some(Prompt::Password));
        assert_eq!(
            classify_prompt("Password required for share media on nas\r\nUser [me]: "),
            Some(Prompt::User)
        );
        assert_eq!(
            classify_prompt("Domain [WORKGROUP]: "),
            Some(Prompt::Domain)
        );
        assert_eq!(
            classify_prompt("Anonymous? (y/n) [n]: "),
            Some(Prompt::Anonymous)
        );
        // A finished line, even one that talks about a password, is not a
        // question.
        assert_eq!(classify_prompt("Password required for share x\r\n"), None);
        assert_eq!(classify_prompt("Mounting smb://nas/media…"), None);
        assert_eq!(classify_prompt(""), None);
    }

    #[test]
    fn answers_follow_the_chosen_login() {
        let registered = Credentials {
            guest: false,
            user: "alice".into(),
            password: "s3cret".into(),
        };
        assert_eq!(registered.answer(Prompt::Anonymous), "n\n");
        assert_eq!(registered.answer(Prompt::User), "alice\n");
        assert_eq!(registered.answer(Prompt::Domain), "\n");
        assert_eq!(registered.answer(Prompt::Password), "s3cret\n");
        let guest = Credentials {
            guest: true,
            ..Credentials::default()
        };
        assert_eq!(guest.answer(Prompt::Anonymous), "y\n");
        assert_eq!(guest.answer(Prompt::Password), "\n");
    }

    #[test]
    fn last_line_start_tracks_the_unfinished_line() {
        assert_eq!(last_line_start("abc"), 0);
        assert_eq!(last_line_start("abc\r\nPassword: "), 5);
    }

    #[test]
    fn gio_errors_are_trimmed_for_people() {
        assert_eq!(
            failure_message("\r\ngio: smb://nas/: Failed to mount Windows share\r\n"),
            "smb://nas/: Failed to mount Windows share"
        );
        assert_eq!(failure_message(""), "The server did not respond.");
        assert!(already_mounted("gio: x: Location is already mounted\n"));
        assert!(!already_mounted("gio: x: Timed out\n"));
    }
}
